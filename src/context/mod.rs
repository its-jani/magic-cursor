pub mod capture;
pub mod screen;

pub use capture::{capture_active_context, get_current_cursor_pos, CapturedContext};
pub use screen::CapturedImage;
