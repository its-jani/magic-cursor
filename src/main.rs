#![windows_subsystem = "windows"]

pub mod ai;
pub mod automation;
pub mod capture;
pub mod config;
pub mod hooks;
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
use automation::UIAutomationExtractor;
use capture::capture_full_screen;
use config::Config;
use hooks::mouse_keyboard::{
    get_current_cursor_pos, HotkeyDefinition, HotkeyManager, TriggerEvent, TriggerSource,
    WiggleDetectorHandle,
};
use tray::{
    TrayIcon, ID_TRAY_EXIT, ID_TRAY_OPEN_CONFIG, ID_TRAY_OPEN_FOLDER, ID_TRAY_TOGGLE_PAUSE,
    ID_TRAY_TRIGGER, WM_TRAY_CALLBACK,
};
use ui::show_magic_overlay;

const PRIMARY_HOTKEY_ID: i32 = 101;
const FALLBACK_HOTKEY_ID: i32 = 102;

static GLOBAL_TX: Mutex<Option<Sender<TriggerEvent>>> = Mutex::new(None);

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            let id = wparam.0 as i32;
            if (id == PRIMARY_HOTKEY_ID || id == FALLBACK_HOTKEY_ID) && !TrayIcon::is_paused() {
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
    // 1. Initialize background Tokio runtime for async LLM and network streaming
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");

    let _rt_guard = rt.enter();

    // 2. Load or create user configuration (%APPDATA%\MagicCursor\config.toml)
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

    // 5. Create System Tray Icon
    let _tray = TrayIcon::new(hwnd, "Magic Cursor (Win+Shift+G / Wiggle)");

    // 6. Register Global Hotkeys
    let mode = config_arc.general.activation_mode.to_lowercase();
    let hotkey_enabled = mode == "hotkey" || mode == "both";
    let wiggle_enabled = mode == "wiggle" || mode == "both";

    let mut hotkey_mgr = HotkeyManager::new(hwnd);
    if hotkey_enabled {
        // Register configured or default hotkey (Win+Shift+G)
        let primary_def = HotkeyDefinition::parse(&config_arc.general.hotkey)
            .unwrap_or_else(HotkeyDefinition::default_magic);
        let _ = hotkey_mgr.register(PRIMARY_HOTKEY_ID, &primary_def);

        // Also register fallback shortcut (Ctrl+Alt+G)
        let fallback_def = HotkeyDefinition::fallback_magic();
        let _ = hotkey_mgr.register(FALLBACK_HOTKEY_ID, &fallback_def);
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

    // 8. Spawn background worker to handle triggers, capture screen, and display overlay
    let worker_config = Arc::clone(&config_arc);
    std::thread::Builder::new()
        .name("magic-cursor-overlay-worker".into())
        .spawn(move || {
            let ai_client = AiClient::new();
            let uia_extractor = UIAutomationExtractor::new();

            while let Ok(event) = trigger_rx.recv() {
                if TrayIcon::is_paused() {
                    continue;
                }

                // A. Instant sub-10ms DXGI Desktop Duplication frame grab
                let captured_frame = capture_full_screen();

                // B. Query Windows UI Automation elements near cursor
                let hotspots = uia_extractor.extract_hotspots_near_cursor(
                    event.cursor_x,
                    event.cursor_y,
                );

                // C. Launch interactive Magic Pointer overlay
                show_magic_overlay(
                    (*worker_config).clone(),
                    captured_frame,
                    hotspots,
                    (event.cursor_x, event.cursor_y),
                    ai_client.clone(),
                );
            }
        })
        .expect("Failed to spawn overlay worker");

    // 9. Standard Windows Message Loop (Puts main thread to 0.0% CPU sleep while idle)
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
