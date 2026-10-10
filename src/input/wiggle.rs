use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_MOUSEMOVE,
    WM_QUIT,
};

use super::{TriggerEvent, TriggerSource};

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
    start_time: Instant,
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
            window_duration: Duration::from_millis(window_ms),
            cooldown_duration: Duration::from_millis(cooldown_ms),
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

        // Check cooldown
        if now.duration_since(self.last_trigger_time) < self.cooldown_duration {
            self.history.clear();
            self.strokes.clear();
            return;
        }

        // Add current sample
        self.history.push_back(MouseSample { timestamp: now, x, y });

        // Clean up samples outside the sliding window
        while let Some(front) = self.history.front() {
            if now.duration_since(front.timestamp) > self.window_duration {
                self.history.pop_front();
            } else {
                break;
            }
        }

        if self.history.len() < 3 {
            return;
        }

        // Analyze horizontal movement direction changes
        // Sensitivity scales threshold: 1 -> 50px, 5 -> 25px, 10 -> 8px
        let stroke_threshold = ((11 - self.sensitivity) as i32) * 5;

        // Recompute strokes from current window
        self.strokes.clear();
        let mut samples_iter = self.history.iter();
        let mut prev = match samples_iter.next() {
            Some(s) => s,
            None => return,
        };

        let mut current_direction: i8 = 0;
        let mut current_distance: i32 = 0;
        let mut stroke_start = prev.timestamp;

        for sample in samples_iter {
            let dx = sample.x - prev.x;
            if dx.abs() >= 2 {
                let dir = if dx > 0 { 1 } else { -1 };
                if current_direction == 0 {
                    current_direction = dir;
                    current_distance = dx.abs();
                    stroke_start = sample.timestamp;
                } else if current_direction == dir {
                    current_distance += dx.abs();
                } else {
                    // Direction reversed!
                    if current_distance >= stroke_threshold {
                        self.strokes.push_back(Stroke {
                            direction: current_direction,
                            distance: current_distance,
                            start_time: stroke_start,
                        });
                    }
                    current_direction = dir;
                    current_distance = dx.abs();
                    stroke_start = sample.timestamp;
                }
            }
            prev = sample;
        }

        if current_distance >= stroke_threshold && current_direction != 0 {
            self.strokes.push_back(Stroke {
                direction: current_direction,
                distance: current_distance,
                start_time: stroke_start,
            });
        }

        // Require at least 4 direction reversals within the window
        if self.strokes.len() >= 4 {
            let first_stroke = self.strokes.front().unwrap();
            let last_stroke = self.strokes.back().unwrap();
            
            if last_stroke.start_time.duration_since(first_stroke.start_time) <= self.window_duration {
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
static IS_HOOK_RUNNING: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn mouse_hook_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if n_code >= 0 && w_param.0 as u32 == WM_MOUSEMOVE {
        let info = *(l_param.0 as *const MSLLHOOKSTRUCT);
        if let Ok(mut state_guard) = WIGGLE_STATE.try_lock() {
            if let Some(ref mut state) = *state_guard {
                state.record_move(info.pt.x, info.pt.y);
            }
        }
    }

    CallNextHookEx(None, n_code, w_param, l_param)
}

pub struct WiggleDetectorHandle {
    thread_id: u32,
}

impl WiggleDetectorHandle {
    pub fn start(
        sensitivity: u32,
        window_ms: u64,
        cooldown_ms: u64,
        tx: Sender<TriggerEvent>,
    ) -> Option<Self> {
        {
            let mut state = WiggleState::new(sensitivity, window_ms, cooldown_ms);
            state.tx = Some(tx);
            let mut guard = WIGGLE_STATE.lock().unwrap();
            *guard = Some(state);
        }

        let (thread_ready_tx, thread_ready_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("magic-wiggle-hook".into())
            .spawn(move || unsafe {
                let thread_id = windows::Win32::System::Threading::GetCurrentThreadId();
                let hook = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(mouse_hook_proc),
                    None,
                    0,
                );

                if let Ok(hook_handle) = hook {
                    IS_HOOK_RUNNING.store(true, Ordering::SeqCst);
                    let _ = thread_ready_tx.send(Some(thread_id));

                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        if msg.message == WM_QUIT {
                            break;
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }

                    let _ = UnhookWindowsHookEx(hook_handle);
                    IS_HOOK_RUNNING.store(false, Ordering::SeqCst);
                } else {
                    let _ = thread_ready_tx.send(None);
                }
            })
            .ok()?;

        let thread_id = thread_ready_rx.recv().ok()??;
        Some(Self { thread_id })
    }

    pub fn set_enabled(enabled: bool) {
        if let Ok(mut guard) = WIGGLE_STATE.lock() {
            if let Some(ref mut state) = *guard {
                state.enabled = enabled;
            }
        }
    }

    pub fn stop(&self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}
