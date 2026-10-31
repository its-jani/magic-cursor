//! Global hotkey registration and the cursor-wiggle gesture detector.
//!
//! Both triggers run on dedicated threads: hotkeys are registered against the
//! message-only window, and the wiggle detector installs a low-level mouse hook
//! on its own thread so a slow hook cannot stall the UI thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN, VK_F1, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, MSG, MSLLHOOKSTRUCT,
    WH_MOUSE_LL, WM_MOUSEMOVE, WM_QUIT,
};

/// Number of mouse samples needed before the detector scores the window.
const MIN_SAMPLES: usize = 4;

/// Number of alternating strokes that make a gesture a wiggle.
const MIN_STROKES: usize = 4;

/// Horizontal movement below this many pixels is treated as jitter and folded
/// into the current stroke instead of starting a new one.
const MIN_STROKE_STEP_PX: i32 = 2;

/// Seconds the detector waits after start-up before the first trigger.
const INITIAL_COOLDOWN_SECS: u64 = 10;

/// Highest virtual key supported by the `F1`-`F24` parser.
const MAX_FUNCTION_KEY: u32 = 24;

/// Which input produced a trigger event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSource {
    /// The registered global hotkey.
    Hotkey,
    /// The cursor wiggle gesture.
    Wiggle,
    /// A click on the tray icon or its menu.
    Tray,
}

/// A request to open the overlay at a screen position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerEvent {
    /// Which input produced the event.
    pub source: TriggerSource,
    /// Cursor x in virtual-screen coordinates.
    pub cursor_x: i32,
    /// Cursor y in virtual-screen coordinates.
    pub cursor_y: i32,
}

/// Returns the current cursor position in virtual-screen coordinates.
pub fn get_current_cursor_pos() -> (i32, i32) {
    let mut point = POINT::default();
    // SAFETY: the point is a stack local written by the API.
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    (point.x, point.y)
}

/// A global hotkey: modifier flags plus a virtual key code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyDefinition {
    /// Bitmask of `MOD_*` flags, including `MOD_NOREPEAT`.
    pub modifiers: u32,
    /// Virtual key code, such as `0x47` for `G`.
    pub vk: u32,
    /// The text the user wrote, echoed back in the UI.
    pub display: String,
}

impl HotkeyDefinition {
    /// The default shortcut, `Win+Shift+G`.
    pub fn default_magic() -> Self {
        Self::with(vk_for_char('G'), MOD_WIN.0 | MOD_SHIFT.0, "Win+Shift+G")
    }

    /// The fallback shortcut, `Ctrl+Alt+G`, registered alongside the primary one.
    pub fn fallback_magic() -> Self {
        Self::with(vk_for_char('G'), MOD_CONTROL.0 | MOD_ALT.0, "Ctrl+Alt+G")
    }

    /// Parses a shortcut such as `Win+Shift+G`, `Ctrl+Alt+F5`, or `Alt+Space`.
    ///
    /// Returns `None` when the text is empty or contains no recognisable key.
    /// Unknown modifier words are ignored rather than failing the whole parse, so
    /// a typo degrades to a shorter shortcut instead of disabling the hotkey.
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut modifiers = MOD_NOREPEAT.0;
        let mut vk = None;

        for part in trimmed.split('+').map(str::trim) {
            let name = part.to_ascii_uppercase();
            match name.as_str() {
                "ALT" | "OPTION" => modifiers |= MOD_ALT.0,
                "CTRL" | "CONTROL" => modifiers |= MOD_CONTROL.0,
                "SHIFT" => modifiers |= MOD_SHIFT.0,
                "WIN" | "WINDOWS" | "SUPER" | "CMD" => modifiers |= MOD_WIN.0,
                _ => {
                    if let Some(key) = parse_virtual_key(&name) {
                        vk = Some(key);
                    }
                }
            }
        }

        Some(Self {
            modifiers,
            vk: vk?,
            display: trimmed.to_string(),
        })
    }

    /// Builds a definition with the mandatory `MOD_NOREPEAT` flag applied.
    fn with(vk: u32, modifiers: u32, display: &str) -> Self {
        Self {
            vk,
            modifiers: modifiers | MOD_NOREPEAT.0,
            display: display.to_string(),
        }
    }
}

/// Returns the virtual key code of an ASCII letter or digit.
fn vk_for_char(character: char) -> u32 {
    character.to_ascii_uppercase() as u32
}

/// Parses a single character or an `F1`-`F24` name into a virtual key code.
fn parse_virtual_key(name: &str) -> Option<u32> {
    if name == "SPACE" {
        return Some(u32::from(VK_SPACE.0));
    }

    let mut characters = name.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        return character
            .is_ascii_alphanumeric()
            .then(|| vk_for_char(character));
    }

    let number = name.strip_prefix('F')?.parse::<u32>().ok()?;
    if !(1..=MAX_FUNCTION_KEY).contains(&number) {
        return None;
    }

    let first = u32::from(VK_F1.0);
    Some(first + (number - 1))
}

/// Registers global hotkeys against a message window and releases them on drop.
#[derive(Debug)]
pub struct HotkeyManager {
    hwnd: HWND,
    registered_ids: Vec<i32>,
}

impl HotkeyManager {
    /// Creates a manager that registers against `hwnd`.
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            registered_ids: Vec::new(),
        }
    }

    /// Registers `definition` under `id`, returning `false` when another
    /// application already owns that combination.
    pub fn register(&mut self, id: i32, definition: &HotkeyDefinition) -> bool {
        // SAFETY: the window handle comes from the message loop that dispatches the
        // WM_HOTKEY messages this registration produces.
        let registered = unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                id,
                HOT_KEY_MODIFIERS(definition.modifiers),
                definition.vk,
            )
        }
        .is_ok();

        if registered {
            self.registered_ids.push(id);
        }
        registered
    }

    /// Unregisters every hotkey this manager owns.
    pub fn unregister_all(&mut self) {
        for id in self.registered_ids.drain(..) {
            // SAFETY: `id` was registered against this window and is unregistered once.
            unsafe {
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

/// One recorded cursor position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MouseSample {
    timestamp: Instant,
    x: i32,
}

/// Direction of a horizontal stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Left,
    Right,
}

/// One continuous horizontal movement of the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stroke {
    direction: Direction,
    distance: i32,
}

/// Detector state, guarded by [`WIGGLE_STATE`] because the hook callback is a
/// bare `extern "system"` function with no user data pointer.
#[derive(Debug)]
struct WiggleState {
    enabled: bool,
    sensitivity: u32,
    window_duration: Duration,
    cooldown_duration: Duration,
    last_trigger: Option<Instant>,
    history: VecDeque<MouseSample>,
    tx: Option<Sender<TriggerEvent>>,
}

impl WiggleState {
    /// Creates a state with clamped sensitivity and timing floors.
    fn new(sensitivity: u32, window_ms: u64, cooldown_ms: u64) -> Self {
        Self {
            enabled: true,
            sensitivity: sensitivity.clamp(1, 10),
            window_duration: Duration::from_millis(window_ms.max(100)),
            cooldown_duration: Duration::from_millis(cooldown_ms.max(500)),
            // Suppresses a trigger from whatever the cursor was doing at startup.
            last_trigger: Instant::now().checked_sub(Duration::from_secs(INITIAL_COOLDOWN_SECS)),
            history: VecDeque::with_capacity(64),
            tx: None,
        }
    }

    /// Minimum horizontal distance, in pixels, that separates two strokes.
    ///
    /// Sensitivity 1 requires 45 px per stroke, sensitivity 10 requires 10 px.
    fn stroke_threshold(&self) -> i32 {
        ((11 - self.sensitivity) as i32) * 5
    }

    /// Returns whether the detector is inside the post-trigger cooldown.
    fn cooling_down(&self, now: Instant) -> bool {
        self.last_trigger
            .is_some_and(|last| now.saturating_duration_since(last) < self.cooldown_duration)
    }

    /// Records a cursor position and emits a trigger when a wiggle completes.
    fn record_move(&mut self, x: i32, y: i32) {
        if !self.enabled {
            return;
        }

        let now = Instant::now();
        if self.cooling_down(now) {
            self.history.clear();
            return;
        }

        self.history.push_back(MouseSample { timestamp: now, x });
        self.prune(now);

        if self.history.len() < MIN_SAMPLES {
            return;
        }

        let strokes = self.strokes_in_window();

        if strokes.len() >= MIN_STROKES && is_alternating(&strokes) {
            self.last_trigger = Some(now);
            self.history.clear();
            self.emit_trigger(x, y);
        }
    }

    /// Drops samples that fell out of the sliding window.
    fn prune(&mut self, now: Instant) {
        while let Some(oldest) = self.history.front() {
            if now.saturating_duration_since(oldest.timestamp) <= self.window_duration {
                break;
            }
            self.history.pop_front();
        }
    }

    /// Groups the sampled positions into strokes longer than the threshold.
    fn strokes_in_window(&self) -> Vec<Stroke> {
        let threshold = self.stroke_threshold();
        let mut strokes = Vec::new();
        let mut samples = self.history.iter();

        let Some(mut previous) = samples.next() else {
            return strokes;
        };

        let mut current: Option<Stroke> = None;

        for sample in samples {
            let dx = sample.x - previous.x;
            previous = sample;

            if dx.abs() < MIN_STROKE_STEP_PX {
                continue;
            }

            let direction = if dx > 0 {
                Direction::Right
            } else {
                Direction::Left
            };

            match current {
                Some(stroke) if stroke.direction == direction => {
                    current = Some(Stroke {
                        direction,
                        distance: stroke.distance + dx.abs(),
                    });
                }
                Some(stroke) => {
                    if stroke.distance >= threshold {
                        strokes.push(stroke);
                    }
                    current = Some(Stroke {
                        direction,
                        distance: dx.abs(),
                    });
                }
                None => {
                    current = Some(Stroke {
                        direction,
                        distance: dx.abs(),
                    });
                }
            }
        }

        if let Some(stroke) = current {
            if stroke.distance >= threshold {
                strokes.push(stroke);
            }
        }

        strokes
    }

    /// Sends the trigger event, ignoring a closed channel.
    fn emit_trigger(&self, x: i32, y: i32) {
        let Some(tx) = &self.tx else {
            return;
        };

        let _ = tx.send(TriggerEvent {
            source: TriggerSource::Wiggle,
            cursor_x: x,
            cursor_y: y,
        });
    }
}

/// Returns whether consecutive strokes alternate direction.
fn is_alternating(strokes: &[Stroke]) -> bool {
    strokes
        .windows(2)
        .all(|pair| pair[0].direction != pair[1].direction)
}

/// Detector state shared with the hook callback.
static WIGGLE_STATE: Mutex<Option<WiggleState>> = Mutex::new(None);

/// Fast path flag checked before taking the state lock on every mouse move.
static WIGGLE_ENABLED: AtomicBool = AtomicBool::new(true);

/// Low-level mouse hook callback installed with `WH_MOUSE_LL`.
///
/// SAFETY: the system calls this with `code >= 0` guarding a valid
/// `MSLLHOOKSTRUCT` behind `lparam`, which is only dereferenced after that check.
unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE && WIGGLE_ENABLED.load(Ordering::Relaxed) {
        let info = lparam.0 as *const MSLLHOOKSTRUCT;

        if !info.is_null() {
            // SAFETY: the Windows hook contract guarantees a valid MSLLHOOKSTRUCT
            // pointer for WH_MOUSE_LL while the callback runs.
            let point = unsafe { (*info).pt };

            // try_lock keeps a stalled worker from blocking the whole mouse.
            if let Ok(mut guard) = WIGGLE_STATE.try_lock() {
                if let Some(state) = guard.as_mut() {
                    state.record_move(point.x, point.y);
                }
            }
        }
    }

    // SAFETY: forwarding is mandatory; there is no ownership of the arguments.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Owns the hook thread; dropping it stops the thread and unhooks the mouse.
#[derive(Debug)]
pub struct WiggleDetectorHandle {
    thread_handle: Option<std::thread::JoinHandle<()>>,
    thread_id: u32,
}

impl WiggleDetectorHandle {
    /// Installs the hook on a dedicated thread and returns once it is live.
    ///
    /// Returns `None` when the thread cannot be spawned or the hook cannot be
    /// installed, in which case the application keeps working on hotkeys alone.
    pub fn start(
        sensitivity: u32,
        window_ms: u64,
        cooldown_ms: u64,
        tx: Sender<TriggerEvent>,
    ) -> Option<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<(u32, bool)>();

        let thread_handle = std::thread::Builder::new()
            .name("magic-cursor-wiggle-hook".to_string())
            .spawn(move || run_hook_thread(sensitivity, window_ms, cooldown_ms, tx, &ready_tx))
            .ok()?;

        let (thread_id, installed) = ready_rx.recv().ok()?;
        if !installed {
            let _ = thread_handle.join();
            return None;
        }

        Some(Self {
            thread_handle: Some(thread_handle),
            thread_id,
        })
    }

    /// Enables or disables the gesture detector without unhooking the mouse.
    pub fn set_enabled(enabled: bool) {
        WIGGLE_ENABLED.store(enabled, Ordering::SeqCst);

        let mut guard = WIGGLE_STATE.lock().expect("wiggle state lock poisoned");
        if let Some(state) = guard.as_mut() {
            state.enabled = enabled;
        }
    }
}

impl Drop for WiggleDetectorHandle {
    fn drop(&mut self) {
        if self.thread_id != 0 {
            // SAFETY: the id belongs to the worker thread, which owns a message
            // queue that this WM_QUIT ends.
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }

        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Body of the hook thread: install the hook, pump messages, then unhook.
fn run_hook_thread(
    sensitivity: u32,
    window_ms: u64,
    cooldown_ms: u64,
    tx: Sender<TriggerEvent>,
    ready: &Sender<(u32, bool)>,
) {
    // SAFETY: GetCurrentThreadId has no preconditions and identifies this thread.
    let thread_id = unsafe { GetCurrentThreadId() };

    // SAFETY: the callback outlives the hook because the hook is removed before
    // this thread returns, and a global hook accepts a null module handle.
    let hook: HHOOK =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), None, 0) }
            .unwrap_or_default();

    if hook.is_invalid() {
        eprintln!("[MagicCursor] Failed to install the low-level mouse hook");
        let _ = ready.send((thread_id, false));
        return;
    }

    let mut state = WiggleState::new(sensitivity, window_ms, cooldown_ms);
    state.tx = Some(tx);
    *WIGGLE_STATE.lock().expect("wiggle state lock poisoned") = Some(state);
    let _ = ready.send((thread_id, true));

    let mut message = MSG::default();
    // SAFETY: standard message pump; the hook thread has its own queue.
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let _ = UnhookWindowsHookEx(hook);
    }

    *WIGGLE_STATE.lock().expect("wiggle state lock poisoned") = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_letters() {
        let hotkey = HotkeyDefinition::parse("Win+Shift+G").expect("valid shortcut");

        assert_eq!(hotkey.vk, vk_for_char('G'));
        assert_eq!(
            hotkey.modifiers,
            MOD_WIN.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0,
            "MOD_NOREPEAT is always added"
        );
    }

    #[test]
    fn parses_aliases_case_insensitively() {
        let control = HotkeyDefinition::parse("control + option + g").expect("valid shortcut");
        let command = HotkeyDefinition::parse("CMD+SUPER+K").expect("valid shortcut");

        assert_eq!(
            control.modifiers,
            MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0
        );
        assert_eq!(command.modifiers, MOD_WIN.0 | MOD_NOREPEAT.0);
        assert_eq!(command.vk, vk_for_char('K'));
    }

    #[test]
    fn parses_function_keys_and_space() {
        let f5 = HotkeyDefinition::parse("Ctrl+F5").expect("valid shortcut");
        let space = HotkeyDefinition::parse("Alt+Space").expect("valid shortcut");

        assert_eq!(f5.vk, u32::from(VK_F1.0) + 4);
        assert_eq!(space.vk, u32::from(VK_SPACE.0));
    }

    #[test]
    fn rejects_input_without_a_key() {
        assert!(HotkeyDefinition::parse("").is_none());
        assert!(HotkeyDefinition::parse("   ").is_none());
        assert!(HotkeyDefinition::parse("Ctrl+Shift").is_none());
        assert!(HotkeyDefinition::parse("F25").is_none());
    }

    fn state_with(samples: &[i32]) -> WiggleState {
        let mut state = WiggleState::new(5, 10_000, 0);
        let now = Instant::now();
        for (offset, x) in samples.iter().enumerate() {
            state.history.push_back(MouseSample {
                timestamp: now + Duration::from_millis(offset as u64),
                x: *x,
            });
        }
        state
    }

    #[test]
    fn alternating_strokes_are_detected() {
        let state = state_with(&[0, 40, -10, 40, -10, 40]);
        let strokes = state.strokes_in_window();

        assert!(strokes.len() >= MIN_STROKES, "got {strokes:?}");
        assert!(is_alternating(&strokes));
    }

    #[test]
    fn one_direction_is_not_a_wiggle() {
        let state = state_with(&[0, 20, 40, 60, 80, 100]);
        let strokes = state.strokes_in_window();

        assert!(!(strokes.len() >= MIN_STROKES && is_alternating(&strokes)));
    }

    #[test]
    fn sensitivity_lowers_the_threshold() {
        let strict = WiggleState::new(1, 300, 0);
        let loose = WiggleState::new(10, 300, 0);

        assert_eq!(strict.stroke_threshold(), 50);
        assert_eq!(loose.stroke_threshold(), 5);
    }

    #[test]
    fn prunes_samples_outside_the_window() {
        let mut state = WiggleState::new(5, 100, 0);
        let now = Instant::now();
        state.history.push_back(MouseSample {
            timestamp: now
                .checked_sub(Duration::from_millis(500))
                .expect("500ms of monotonic time must have elapsed"),
            x: 1,
        });
        state.history.push_back(MouseSample {
            timestamp: now,
            x: 2,
        });

        state.prune(now);

        assert_eq!(state.history.len(), 1);
        assert_eq!(state.history[0].x, 2);
    }
}
