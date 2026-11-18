//! Native Win32 message boxes for failures the tray app cannot log anywhere
//! visible: a `windows`-subsystem process has no console, so stderr is lost.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    MESSAGEBOX_STYLE,
};

/// Shows a fatal error the user must acknowledge before the app gives up.
pub fn show_error(message: &str) {
    show(message, MB_ICONERROR);
}

/// Shows a recoverable problem; the application keeps running afterwards.
pub fn show_warning(message: &str) {
    show(message, MB_ICONWARNING);
}

/// Displays `message` in a foreground message box titled "Magic Cursor".
fn show(message: &str, icon: MESSAGEBOX_STYLE) {
    let text = wide_null(message);
    let title = wide_null("Magic Cursor");

    // SAFETY: both strings are NUL-terminated and live until the call returns;
    // a null owner handle is allowed for MessageBoxW.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_SETFOREGROUND | MB_TOPMOST | icon,
        );
    }
}

/// Encodes a Rust string as a NUL-terminated UTF-16 buffer for Win32.
fn wide_null(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_buffers_are_nul_terminated() {
        let buffer = wide_null("Magic Cursor");

        assert_eq!(buffer.last(), Some(&0), "Win32 requires a trailing NUL");
        assert_eq!(buffer.len(), "Magic Cursor".len() + 1);
    }
}
