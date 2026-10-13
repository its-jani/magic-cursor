use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, MSG, MSLLHOOKSTRUCT,
    WH_MOUSE_LL, WM_MOUSEMOVE, WM_QUIT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSource {
    Hotkey,
    Wiggle,
    Tray,
    Manual,
}

#[derive(Debug, Clone)]
pub struct TriggerEvent {
    pub source: TriggerSource,
    pub cursor_x: i32,
    pub cursor_y: i32,
}

pub fn get_current_cursor_pos() -> (i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        (pt.x, pt.y)
    }
}

// =========================================================================
// Global Hotkey Management (Win + Shift + G / Ctrl + Alt + G / Configurable)
// =========================================================================

#[derive(Debug, Clone)]
pub struct HotkeyDefinition {
    pub modifiers: u32,
    pub vk: u32,
    pub display: String,
}

impl HotkeyDefinition {
    pub fn default_magic() -> Self {
        // Win + Shift + G
        Self {
            modifiers: (MOD_WIN.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0),
            vk: b'G' as u32,
            display: "Win+Shift+G".to_string(),
        }
    }

    pub fn fallback_magic() -> Self {
        // Ctrl + Alt + G
        Self {
            modifiers: (MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0),
            vk: b'G' as u32,
            display: "Ctrl+Alt+G".to_string(),
        }
    }

    pub fn parse(hotkey_str: &str) -> Option<Self> {
        let trimmed = hotkey_str.trim();
        if trimmed.is_empty() {
            return None;
        }

        let parts: Vec<&str> = trimmed.split('+').map(|s| s.trim()).collect();
        let mut modifiers: u32 = MOD_NOREPEAT.0;
        let mut vk: Option<u32> = None;

        for part in parts {
            let p_upper = part.to_uppercase();
            match p_upper.as_str() {
                "ALT" | "OPTION" => modifiers |= MOD_ALT.0,
                "CTRL" | "CONTROL" => modifiers |= MOD_CONTROL.0,
                "SHIFT" => modifiers |= MOD_SHIFT.0,
                "WIN" | "WINDOWS" | "SUPER" | "CMD" => modifiers |= MOD_WIN.0,
                "SPACE" => vk = Some(VK_SPACE.0 as u32),
                letter if letter.len() == 1 => {
                    let ch = letter.chars().next().unwrap();
                    if ch.is_ascii_alphanumeric() {
                        vk = Some(ch.to_ascii_uppercase() as u32);
                    }
                }
                fn_key if fn_key.starts_with('F') && fn_key.len() <= 3 => {
                    if let Ok(num) = fn_key[1..].parse::<u32>() {
                        if (1..=24).contains(&num) {
                            // VK_F1 is 0x70
                            vk = Some(0x70 + (num - 1));
                        }
                    }
                }
                _ => {}
            }
        }

        let key = vk?;
        Some(Self {
            modifiers,
            vk: key,
            display: hotkey_str.to_string(),
        })
    }
}

pub struct HotkeyManager {
    hwnd: HWND,
    registered_ids: Vec<i32>,
}

impl HotkeyManager {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            registered_ids: Vec::new(),
        }
    }

    pub fn register(&mut self, id: i32, def: &HotkeyDefinition) -> bool {
        unsafe {
            let res = RegisterHotKey(
                Some(self.hwnd),
                id,
                HOT_KEY_MODIFIERS(def.modifiers),
                def.vk,
            );
            if res.is_ok() {
                self.registered_ids.push(id);
                true
            } else {
                false
            }
        }
    }

    pub fn unregister_all(&mut self) {
        unsafe {
            for id in self.registered_ids.drain(..) {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
            }
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
    }
}

// =========================================================================
// Cursor Wiggle Detector (Low-level SetWindowsHookExW WH_MOUSE_LL)
// =========================================================================

#[allow(dead_code)]
struct MouseSample {
    timestamp: Instant,
    x: i32,
    y: i32,
}

#[allow(dead_code)]
struct Stroke {
    direction: i8, // -1 for left, +1 for right
    distance: i32,
}

struct WiggleState {
    enabled: bool,
    sensitivity: u32,
    window_duration: Duration,
    cooldown_duration: Duration,
    last_trigger_time: Instant,
    history: VecDeque<MouseSample>,
    strokes: VecDeque<Stroke>,
    tx: Option<Sender<TriggerEvent>>,
}

impl WiggleState {
    fn new(sensitivity: u32, window_ms: u64, cooldown_ms: u64) -> Self {
        let now = Instant::now();
        Self {
            enabled: true,
            sensitivity: sensitivity.clamp(1, 10),
            window_duration: Duration::from_millis(window_ms.max(100)),
            cooldown_duration: Duration::from_millis(cooldown_ms.max(500)),
            last_trigger_time: now - Duration::from_secs(10),
            history: VecDeque::with_capacity(64),
            strokes: VecDeque::with_capacity(16),
            tx: None,
        }
    }

    fn record_move(&mut self, x: i32, y: i32) {
        if !self.enabled {
            return;
        }

        let now = Instant::now();
        if now.duration_since(self.last_trigger_time) < self.cooldown_duration {
            self.history.clear();
            self.strokes.clear();
            return;
        }

        self.history.push_back(MouseSample { timestamp: now, x, y });

        // Retain only samples within sliding time window (default 300ms)
        while let Some(front) = self.history.front() {
            if now.duration_since(front.timestamp) > self.window_duration {
                self.history.pop_front();
            } else {
                break;
            }
        }

        if self.history.len() < 4 {
            return;
        }

        // Sensitivity: 1 (requires 45px per stroke) to 10 (requires 10px per stroke)
        let stroke_threshold = ((11 - self.sensitivity) as i32) * 5;

        // Recompute strokes across samples
        self.strokes.clear();
        let mut samples_iter = self.history.iter();
        let mut prev = match samples_iter.next() {
            Some(s) => s,
            None => return,
        };

        let mut current_direction: i8 = 0;
        let mut current_distance: i32 = 0;

        for cur in samples_iter {
            let dx = cur.x - prev.x;
            if dx.abs() >= 2 {
                let dir = if dx > 0 { 1 } else { -1 };
                if current_direction == 0 {
                    current_direction = dir;
                    current_distance = dx.abs();
                } else if current_direction == dir {
                    current_distance += dx.abs();
                } else {
                    if current_distance >= stroke_threshold {
                        self.strokes.push_back(Stroke {
                            direction: current_direction,
                            distance: current_distance,
                        });
                    }
                    current_direction = dir;
                    current_distance = dx.abs();
                }
            }
            prev = cur;
        }

        if current_distance >= stroke_threshold {
            self.strokes.push_back(Stroke {
                direction: current_direction,
                distance: current_distance,
            });
        }

        // Require at least 4 alternating strokes within the window
        if self.strokes.len() >= 4 {
            let mut alternating = true;
            let mut prev_dir = self.strokes[0].direction;

            for s in self.strokes.iter().skip(1) {
                if s.direction == prev_dir {
                    alternating = false;
                    break;
                }
                prev_dir = s.direction;
            }

            if alternating {
                self.last_trigger_time = now;
                self.history.clear();
                self.strokes.clear();

                if let Some(ref tx) = self.tx {
                    let _ = tx.send(TriggerEvent {
                        source: TriggerSource::Wiggle,
                        cursor_x: x,
                        cursor_y: y,
                    });
                }
            }
        }
    }
}

static WIGGLE_STATE: Mutex<Option<WiggleState>> = Mutex::new(None);
static IS_ENABLED_ATOMIC: AtomicBool = AtomicBool::new(true);

unsafe extern "system" fn low_level_mouse_proc(
    n_code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if n_code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE {
        if IS_ENABLED_ATOMIC.load(Ordering::Relaxed) {
            let hook_struct = *(lparam.0 as *const MSLLHOOKSTRUCT);
            let pt_x = hook_struct.pt.x;
            let pt_y = hook_struct.pt.y;

            if let Ok(mut guard) = WIGGLE_STATE.try_lock() {
                if let Some(ref mut state) = *guard {
                    state.record_move(pt_x, pt_y);
                }
            }
        }
    }

    CallNextHookEx(None, n_code, wparam, lparam)
}

pub struct WiggleDetectorHandle {
    thread_handle: Option<std::thread::JoinHandle<()>>,
    thread_id: u32,
}

impl WiggleDetectorHandle {
    pub fn start(
        sensitivity: u32,
        window_ms: u64,
        cooldown_ms: u64,
        tx: Sender<TriggerEvent>,
    ) -> Option<Self> {
        let (init_tx, init_rx) = std::sync::mpsc::channel::<(u32, bool)>();

        let thread_handle = std::thread::Builder::new()
            .name("magic-cursor-wiggle-hook".into())
            .spawn(move || {
                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };

                let hook_res: Result<HHOOK, _> = unsafe {
                    SetWindowsHookExW(
                        WH_MOUSE_LL,
                        Some(low_level_mouse_proc),
                        None,
                        0,
                    )
                };

                let hook = match hook_res {
                    Ok(h) => {
                        let mut state = WiggleState::new(sensitivity, window_ms, cooldown_ms);
                        state.tx = Some(tx);
                        if let Ok(mut guard) = WIGGLE_STATE.lock() {
                            *guard = Some(state);
                        }
                        let _ = init_tx.send((thread_id, true));
                        h
                    }
                    Err(e) => {
                        eprintln!("[MagicCursor] Failed to install low-level mouse hook: {:?}", e);
                        let _ = init_tx.send((thread_id, false));
                        return;
                    }
                };

                unsafe {
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        if msg.message == WM_QUIT {
                            break;
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }

                    let _ = UnhookWindowsHookEx(hook);
                }

                if let Ok(mut guard) = WIGGLE_STATE.lock() {
                    *guard = None;
                }
            })
            .ok()?;

        let (thread_id, success) = init_rx.recv().ok()?;
        if !success {
            return None;
        }

        Some(Self {
            thread_handle: Some(thread_handle),
            thread_id,
        })
    }

    pub fn set_enabled(enabled: bool) {
        IS_ENABLED_ATOMIC.store(enabled, Ordering::SeqCst);
        if let Ok(mut guard) = WIGGLE_STATE.lock() {
            if let Some(ref mut state) = *guard {
                state.enabled = enabled;
            }
        }
    }
}

impl Drop for WiggleDetectorHandle {
    fn drop(&mut self) {
        if self.thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}
