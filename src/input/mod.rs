pub mod hotkey;
pub mod wiggle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSource {
    Hotkey,
    Wiggle,
    Tray,
}

#[derive(Debug, Clone, Copy)]
pub struct TriggerEvent {
    pub source: TriggerSource,
    pub cursor_x: i32,
    pub cursor_y: i32,
}
