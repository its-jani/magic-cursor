#![windows_subsystem = "windows"]

pub mod ai;
pub mod config;
pub mod context;
pub mod input;
pub mod tray;
pub mod ui;

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, MSG, WINDOW_EX_STYLE,
    WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WM_QUIT, WM_RBUTTONUP, WNDCLASSW,
    WS_OVERLAPPED,
};

use ai::AiClient;
use config::Config;
use context::{capture_active_context, get_current_cursor_pos};
use input::hotkey::{HotkeyDefinition, HotkeyManager};
use input::wiggle::WiggleDetectorHandle;
use input::{TriggerEvent, TriggerSource};
use tray::{
    TrayIcon, ID_TRAY_EXIT, ID_TRAY_OPEN_CONFIG, ID_TRAY_OPEN_FOLDER, ID_TRAY_TOGGLE_PAUSE,
    ID_TRAY_TRIGGER, WM_TRAY_CALLBACK,
};
use ui::show_overlay_window;

const HOTKEY_ID: i32 = 101;

static GLOBAL_TX: Mutex<Option<Sender<TriggerEvent>>> = Mutex::new(None);

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if (wparam.0 as i32) == HOTKEY_ID && !TrayIcon::is_paused() {
                let (x, y) = get_current_cursor_pos();
                if let Ok(guard) = GLOBAL_TX.lock() {
                    if let Some(ref tx) = *guard {
                        let _ = tx.send(TriggerEvent {
                            source: TriggerSource::Hotkey,
                            cursor_x: x,
                            cursor_y: y,
                        });
                    }
                }
            }
            LRESULT(0)
        }
        WM_TRAY_CALLBACK => {
            let event = lparam.0 as u32;
            if event == WM_RBUTTONUP {
                TrayIcon::show_context_menu(hwnd);
            } else if event == WM_LBUTTONUP && !TrayIcon::is_paused() {
                let (x, y) = get_current_cursor_pos();
                if let Ok(guard) = GLOBAL_TX.lock() {
                    if let Some(ref tx) = *guard {
                        let _ = tx.send(TriggerEvent {
                            source: TriggerSource::Tray,
                            cursor_x: x,
                            cursor_y: y,
                        });
                    }
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd = wparam.0 as usize;
            match cmd {
                ID_TRAY_TRIGGER => {
                    let (x, y) = get_current_cursor_pos();
                    if let Ok(guard) = GLOBAL_TX.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(TriggerEvent {
                                source: TriggerSource::Tray,
                                cursor_x: x,
                                cursor_y: y,
                            });
                        }
                    }
                }
                ID_TRAY_TOGGLE_PAUSE => {
                    let current = TrayIcon::is_paused();
                    TrayIcon::set_paused(!current);
                    WiggleDetectorHandle::set_enabled(current);
                }
                ID_TRAY_OPEN_CONFIG => {
                    let path = Config::config_path();
                    let _ = open_file_in_explorer(&path);
                }
                ID_TRAY_OPEN_FOLDER => {
                    let dir = Config::config_dir();
                    let _ = open_file_in_explorer(&dir);
                }
                ID_TRAY_EXIT => {
                    let _ = DestroyWindow(hwnd);
                    PostQuitMessage(0);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn open_file_in_explorer(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::process::Command;
        Command::new("explorer").arg(path).spawn()?;
    }
    Ok(())
}

fn main() {
    // 1. Initialize background Tokio runtime for async LLM calls
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");

    let _rt_guard = rt.enter();

    // 2. Load or create config
    let (config, _config_path) = Config::load_or_create();
    let config_arc = Arc::new(config);

    // 3. Setup trigger event channel
    let (trigger_tx, trigger_rx) = channel::<TriggerEvent>();
    if let Ok(mut guard) = GLOBAL_TX.lock() {
        *guard = Some(trigger_tx.clone());
    }

    // 4. Create Win32 message-only window
    let class_name = w!("MagicCursorMsgWindowClass");
    let hwnd = unsafe {
        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(std::ptr::null_mut()),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wnd_class);

        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("MagicCursorBackgroundWindow"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
        .expect("Failed to create background message window")
    };

    // 5. Create Tray Icon
    let _tray = TrayIcon::new(hwnd, "MagicCursor-Win (Alt+G)");

    // 6. Register Hotkey
    let mode = config_arc.general.activation_mode.to_lowercase();
    let hotkey_enabled = mode == "hotkey" || mode == "both";
    let wiggle_enabled = mode == "wiggle" || mode == "both";

    let mut hotkey_mgr = HotkeyManager::new(hwnd, HOTKEY_ID);
    if hotkey_enabled {
        if let Some(def) = HotkeyDefinition::parse(&config_arc.general.hotkey) {
            let registered = hotkey_mgr.register(&def);
            if !registered {
                eprintln!(
                    "[MagicCursor] Warning: Failed to register hotkey '{}'",
                    config_arc.general.hotkey
                );
            }
        }
    }

    // 7. Start Mouse Wiggle Detector if configured
    let _wiggle_handle = if wiggle_enabled {
        WiggleDetectorHandle::start(
            config_arc.general.wiggle_sensitivity,
            config_arc.general.wiggle_window_ms,
            config_arc.general.wiggle_cooldown_ms,
            trigger_tx.clone(),
        )
    } else {
        None
    };

    // 8. Spawn background worker to handle triggers and display HUD overlay
    let worker_config = Arc::clone(&config_arc);
    std::thread::Builder::new()
        .name("magic-overlay-worker".into())
        .spawn(move || {
            let ai_client = AiClient::new();

            while let Ok(event) = trigger_rx.recv() {
                if TrayIcon::is_paused() {
                    continue;
                }

                // Capture clipboard or screen context around cursor
                let context = capture_active_context(
                    &worker_config,
                    event.cursor_x,
                    event.cursor_y,
                );

                // Show eframe overlay window
                show_overlay_window(
                    (*worker_config).clone(),
                    context,
                    (event.cursor_x, event.cursor_y),
                    ai_client.clone(),
                );
            }
        })
        .expect("Failed to spawn overlay worker");

    // 9. Standard Windows Message Loop (Puts thread to 0% CPU sleep while idle)
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_QUIT {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
