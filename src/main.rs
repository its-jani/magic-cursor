//! `MagicCursor`: a screen-aware AI overlay that lives under the cursor.
//!
//! The process runs two threads. The main thread owns the Win32 message pump,
//! the tray icon, and the global hotkeys, which keeps idle CPU at zero because
//! `GetMessageW` blocks. A worker thread receives trigger events, captures the
//! desktop, extracts UI Automation hotspots, and runs the overlay on the main
//! thread once a frame is ready.

#![windows_subsystem = "windows"]

pub mod ai;
pub mod automation;
pub mod capture;
pub mod config;
pub mod hooks;
pub mod tray;
pub mod ui;

use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, TranslateMessage, MSG, WINDOW_EX_STYLE, WM_COMMAND, WM_DESTROY, WM_HOTKEY,
    WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

use ai::AiClient;
use automation::UIAutomationExtractor;
use capture::capture_full_screen;
use config::Config;
use hooks::mouse_keyboard::{
    get_current_cursor_pos, HotkeyDefinition, HotkeyManager, TriggerEvent, TriggerSource,
};
use hooks::WiggleDetectorHandle;
use tray::{
    TrayIcon, ID_TRAY_EXIT, ID_TRAY_OPEN_CONFIG, ID_TRAY_OPEN_FOLDER, ID_TRAY_TOGGLE_PAUSE,
    ID_TRAY_TRIGGER, WM_TRAY_CALLBACK,
};
use ui::{show_magic_overlay, OverlayRequest};

/// Hotkey id of the user-configured shortcut.
const PRIMARY_HOTKEY_ID: i32 = 101;
/// Hotkey id of the `Ctrl+Alt+G` fallback.
const FALLBACK_HOTKEY_ID: i32 = 102;

/// Thread name of the worker that captures and shows the overlay.
const WORKER_THREAD_NAME: &str = "magic-cursor-overlay-worker";

/// Sink for trigger events; `None` until `main` installs the channel.
static TRIGGER_SENDER: Mutex<Option<Sender<TriggerEvent>>> = Mutex::new(None);

/// Window procedure of the message-only window that receives hotkey and tray events.
///
/// SAFETY: the system calls this with messages and parameters that match
/// `WNDCLASSW` registration; anything unrecognised is forwarded to
/// `DefWindowProcW`, which is what makes the parameter interpretation sound.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_HOTKEY => {
            let id = wparam.0 as i32;
            let is_trigger_hotkey = id == PRIMARY_HOTKEY_ID || id == FALLBACK_HOTKEY_ID;

            if is_trigger_hotkey && TrayIcon::pause_state().is_running() {
                send_trigger(TriggerSource::Hotkey);
            }

            LRESULT(0)
        }
        WM_TRAY_CALLBACK => {
            let event = lparam.0 as u32;

            if event == WM_RBUTTONUP {
                TrayIcon::show_context_menu(hwnd);
            } else if event == WM_LBUTTONUP && TrayIcon::pause_state().is_running() {
                send_trigger(TriggerSource::Tray);
            }

            LRESULT(0)
        }
        WM_COMMAND => {
            handle_menu_command(wparam.0, hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: posts the quit message for this thread's message loop.
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => {
            // SAFETY: unhandled messages are forwarded to the default procedure,
            // which is the documented contract for `WNDCLASSW::lpfnWndProc`.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

/// Dispatches a tray menu command.
fn handle_menu_command(command: usize, hwnd: HWND) {
    match command {
        ID_TRAY_TRIGGER => send_trigger(TriggerSource::Tray),
        ID_TRAY_TOGGLE_PAUSE => toggle_pause(),
        ID_TRAY_OPEN_CONFIG => open_in_explorer(&Config::config_path()),
        ID_TRAY_OPEN_FOLDER => open_in_explorer(&Config::config_dir()),
        ID_TRAY_EXIT => exit(hwnd),
        _ => {}
    }
}

/// Flips the pause state for both the tray and the wiggle detector.
fn toggle_pause() {
    let next = TrayIcon::pause_state().toggled();
    TrayIcon::set_pause_state(next);
    WiggleDetectorHandle::set_enabled(next.is_running());
}

/// Destroys the message window, which ends the message loop.
fn exit(hwnd: HWND) {
    // SAFETY: `hwnd` is the window this procedure belongs to, and the quit
    // message terminates the loop that owns it.
    unsafe {
        let _ = DestroyWindow(hwnd);
        PostQuitMessage(0);
    }
}

/// Sends a trigger event to the worker thread, if it is still listening.
fn send_trigger(source: TriggerSource) {
    let (cursor_x, cursor_y) = get_current_cursor_pos();

    let sender = TRIGGER_SENDER.lock().expect("trigger sender lock poisoned");
    if let Some(tx) = sender.as_ref() {
        let _ = tx.send(TriggerEvent {
            source,
            cursor_x,
            cursor_y,
        });
    }
}

/// Opens a path in File Explorer.
fn open_in_explorer(path: &Path) {
    if let Err(err) = std::process::Command::new("explorer").arg(path).spawn() {
        eprintln!("[magic-cursor] could not open {}: {err}", path.display());
    }
}

/// Runs the overlay once per trigger event on a dedicated worker thread.
fn spawn_overlay_worker(config: Arc<Config>, receiver: std::sync::mpsc::Receiver<TriggerEvent>) {
    let spawned = std::thread::Builder::new()
        .name(WORKER_THREAD_NAME.to_string())
        .spawn(move || {
            let ai_client = AiClient::new();
            let extractor = UIAutomationExtractor::new();

            while let Ok(event) = receiver.recv() {
                if !TrayIcon::pause_state().is_running() {
                    continue;
                }

                // Capture first: the frame has to match the screen the user saw
                // when the trigger fired.
                let frame = capture_full_screen();
                let hotspots =
                    extractor.extract_hotspots_near_cursor(event.cursor_x, event.cursor_y);

                show_magic_overlay(OverlayRequest {
                    config: (*config).clone(),
                    frame,
                    hotspots,
                    cursor: (event.cursor_x, event.cursor_y),
                    ai_client: ai_client.clone(),
                });
            }
        });

    if let Err(err) = spawned {
        eprintln!("[magic-cursor] could not spawn the overlay worker: {err}");
    }
}

/// Creates the hidden message window that receives hotkey and tray callbacks.
fn create_message_window() -> windows::core::Result<HWND> {
    let class_name = w!("MagicCursorMsgWindowClass");

    // SAFETY: the class is registered before the window is created, and both use
    // the same static class name pointer for the lifetime of the process.
    unsafe {
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(std::ptr::null_mut()),
            lpszClassName: class_name,
            ..WNDCLASSW::default()
        };
        RegisterClassW(&window_class);

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
    }
}

fn main() {
    // 1. Background Tokio runtime for the streaming LLM calls.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Tokio runtime must start before any AI request is made");
    let _runtime_guard = runtime.enter();

    // 2. User configuration, created on first run.
    let (config, config_path) = Config::load_or_create();
    let config = Arc::new(config);

    // 3. Trigger channel shared with the window procedure.
    let (trigger_tx, trigger_rx) = channel::<TriggerEvent>();
    *TRIGGER_SENDER.lock().expect("trigger sender lock poisoned") = Some(trigger_tx.clone());

    // 4. Hidden message window.
    let hwnd = create_message_window().expect("message window must be created");
    eprintln!(
        "[magic-cursor] loaded configuration from {}",
        config_path.display()
    );

    // 5. Tray icon, kept alive for the lifetime of the message loop.
    let _tray = TrayIcon::new(hwnd, "Magic Cursor (Win+Shift+G / Wiggle)");

    // 6. Global hotkeys.
    let mut hotkeys = HotkeyManager::new(hwnd);
    if config.general.activation_mode.uses_hotkey() {
        let primary = HotkeyDefinition::parse(&config.general.hotkey)
            .unwrap_or_else(HotkeyDefinition::default_magic);
        if !hotkeys.register(PRIMARY_HOTKEY_ID, &primary) {
            eprintln!(
                "[magic-cursor] {} is already owned by another application",
                primary.display
            );
        }

        let fallback = HotkeyDefinition::fallback_magic();
        let _ = hotkeys.register(FALLBACK_HOTKEY_ID, &fallback);
    }

    // 7. Cursor wiggle detector.
    let _wiggle = if config.general.activation_mode.uses_wiggle() {
        WiggleDetectorHandle::start(
            config.general.wiggle_sensitivity,
            config.general.wiggle_window_ms,
            config.general.wiggle_cooldown_ms,
            trigger_tx,
        )
    } else {
        None
    };

    // 8. Overlay worker.
    spawn_overlay_worker(Arc::clone(&config), trigger_rx);

    // 9. Message loop: blocks while idle, so the process costs no CPU.
    let mut message = MSG::default();
    // SAFETY: standard Win32 pump; `message` is a stack local owned by this frame.
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
