//! System tray icon and its context menu.

use std::ffi::OsStr;
use std::fmt;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, SetForegroundWindow,
    TrackPopupMenu, HICON, IDI_APPLICATION, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_USER,
};

/// Callback message the shell posts to the message window for tray events.
pub const WM_TRAY_CALLBACK: u32 = WM_USER + 101;

/// Menu command id that requests an immediate capture.
pub const ID_TRAY_TRIGGER: usize = 1001;
/// Menu command id that toggles pause.
pub const ID_TRAY_TOGGLE_PAUSE: usize = 1002;
/// Menu command id that opens `config.toml`.
pub const ID_TRAY_OPEN_CONFIG: usize = 1003;
/// Menu command id that opens the configuration folder.
pub const ID_TRAY_OPEN_FOLDER: usize = 1004;
/// Menu command id that exits the application.
pub const ID_TRAY_EXIT: usize = 1005;

/// Identifier of the icon inside the message window, used by the shell callbacks.
const ICON_UID: u32 = 1;

/// Whether the application currently responds to triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PauseState {
    /// Triggers open the overlay.
    #[default]
    Running,
    /// Triggers are recorded but ignored.
    Paused,
}

impl PauseState {
    /// Returns whether triggers are currently accepted.
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Returns the opposite state, which is what the tray toggle applies.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Running => Self::Paused,
            Self::Paused => Self::Running,
        }
    }
}

/// Live tray icon; removing the icon happens on drop.
pub struct TrayIcon {
    data: NOTIFYICONDATAW,
}

impl fmt::Debug for TrayIcon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrayIcon")
            .field("icon_id", &ICON_UID)
            .finish_non_exhaustive()
    }
}

/// Owns the popup menu so it is destroyed on every path.
struct PopupMenu(windows::Win32::UI::WindowsAndMessaging::HMENU);

impl Drop for PopupMenu {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreatePopupMenu and is destroyed once.
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

/// Global pause flag, read by both the message window and the overlay worker.
static IS_PAUSED: AtomicBool = AtomicBool::new(false);

impl TrayIcon {
    /// Adds the tray icon for `hwnd`, returning `None` if the shell rejects it.
    pub fn new(hwnd: HWND, tooltip: &str) -> Option<Self> {
        // SAFETY: the stock application icon is owned by the system, so it is never
        // freed here, and the shell copies the icon data it needs.
        let hicon: HICON = unsafe { LoadIconW(None, IDI_APPLICATION).ok()? };

        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY_CALLBACK,
            hIcon: hicon,
            ..NOTIFYICONDATAW::default()
        };

        let wide_tooltip: Vec<u16> = OsStr::new(tooltip)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // `szTip` is a fixed-size array with the same semantics as a C string.
        let copied = wide_tooltip.len().min(data.szTip.len());
        data.szTip[..copied].copy_from_slice(&wide_tooltip[..copied]);

        // SAFETY: `data` is fully initialised and lives until the icon is removed.
        (unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool()).then_some(Self { data })
    }

    /// Records the pause state seen by every trigger source.
    pub fn set_pause_state(state: PauseState) {
        IS_PAUSED.store(state == PauseState::Paused, Ordering::SeqCst);
    }

    /// Returns the current pause state.
    pub fn pause_state() -> PauseState {
        if IS_PAUSED.load(Ordering::SeqCst) {
            PauseState::Paused
        } else {
            PauseState::Running
        }
    }

    /// Shows the context menu at the cursor.
    ///
    /// The menu is modal: this call blocks until the user picks an item or
    /// dismisses it, which is why it runs on the message thread.
    pub fn show_context_menu(hwnd: HWND) {
        let mut point = POINT::default();
        // SAFETY: the point is a stack local written by the API.
        unsafe {
            let _ = GetCursorPos(&mut point);
        }

        // SAFETY: CreatePopupMenu has no preconditions; the wrapper destroys it.
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
            return;
        };
        let menu = PopupMenu(menu);

        let paused = matches!(Self::pause_state(), PauseState::Paused);

        let pause_label = if paused {
            w!("▶ Resume Triggers")
        } else {
            w!("⏸ Pause Triggers")
        };
        let pause_flags = if paused {
            MF_CHECKED | MF_STRING
        } else {
            MF_UNCHECKED | MF_STRING
        };

        // SAFETY: `menu` is a live popup menu and every id is a `usize` this module
        // handles in the message loop.
        unsafe {
            let _ = AppendMenuW(
                menu.0,
                MF_STRING,
                ID_TRAY_TRIGGER,
                w!("✨ Trigger MagicCursor"),
            );
            let _ = AppendMenuW(menu.0, pause_flags, ID_TRAY_TOGGLE_PAUSE, pause_label);
            let _ = AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(
                menu.0,
                MF_STRING,
                ID_TRAY_OPEN_CONFIG,
                w!("⚙ Open config.toml"),
            );
            let _ = AppendMenuW(
                menu.0,
                MF_STRING,
                ID_TRAY_OPEN_FOLDER,
                w!("📁 Open Config Folder"),
            );
            let _ = AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu.0, MF_STRING, ID_TRAY_EXIT, w!("❌ Exit MagicCursor"));

            // Without the foreground window the menu would not dismiss on click-away.
            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(
                menu.0,
                TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                hwnd,
                None,
            );
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // SAFETY: the icon was added with NIM_ADD and is removed exactly once.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_round_trips() {
        assert_eq!(PauseState::Running.toggled(), PauseState::Paused);
        assert_eq!(PauseState::Paused.toggled(), PauseState::Running);
    }

    #[test]
    fn only_running_accepts_triggers() {
        assert!(PauseState::Running.is_running());
        assert!(!PauseState::Paused.is_running());
    }
}
