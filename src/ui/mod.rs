//! egui front end: the overlay window, its theme, and the markdown renderer.

pub mod markdown;
pub mod overlay;
pub mod theme;

pub use overlay::{
    clear_draft, get_saved_draft, save_draft, show_magic_overlay, DraftState, OverlayRequest,
};
