//! Desktop frame capture through DXGI, with a GDI fallback for the cases where
//! desktop duplication is unavailable (exclusive fullscreen, remote sessions,
//! or a driver that refuses a hardware device).

pub mod dxgi;

pub use dxgi::{
    capture_full_screen, capture_region, capture_screen_gdi, CapturedFrame, DxgiCapturer,
    VirtualScreen,
};
