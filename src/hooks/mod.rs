//! Input triggers: the global hotkey and the cursor wiggle gesture.

pub mod mouse_keyboard;

pub use mouse_keyboard::{
    get_current_cursor_pos, HotkeyDefinition, HotkeyManager, TriggerEvent, TriggerSource,
    WiggleDetectorHandle,
};
