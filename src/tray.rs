use std::ffi::OsStr;
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

pub const WM_TRAY_CALLBACK: u32 = WM_USER + 101;
pub const ID_TRAY_TRIGGER: usize = 1001;
pub const ID_TRAY_TOGGLE_PAUSE: usize = 1002;
pub const ID_TRAY_OPEN_CONFIG: usize = 1003;
pub const ID_TRAY_OPEN_FOLDER: usize = 1004;
pub const ID_TRAY_EXIT: usize = 1005;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Trigger,
    TogglePause,
    OpenConfig,
    OpenFolder,
    Exit,
}

pub struct TrayIcon {
    #[allow(dead_code)]
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
}

static IS_PAUSED: AtomicBool = AtomicBool::new(false);

impl TrayIcon {
    pub fn new(hwnd: HWND, tooltip: &str) -> Option<Self> {
        unsafe {
            let hicon: HICON = LoadIconW(None, IDI_APPLICATION).ok()?;

            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY_CALLBACK;
            nid.hIcon = hicon;

            let wide_tip: Vec<u16> = OsStr::new(tooltip)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let len = wide_tip.len().min(nid.szTip.len());
            nid.szTip[..len].copy_from_slice(&wide_tip[..len]);

            let added = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
            if added {
                Some(Self { hwnd, nid })
            } else {
                None
            }
        }
    }

    pub fn set_paused(paused: bool) {
        IS_PAUSED.store(paused, Ordering::SeqCst);
    }

    pub fn is_paused() -> bool {
        IS_PAUSED.load(Ordering::SeqCst)
    }

    pub fn show_context_menu(hwnd: HWND) {
        unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);

            let menu = match CreatePopupMenu() {
                Ok(m) => m,
                Err(_) => return,
            };

            let is_paused = IS_PAUSED.load(Ordering::SeqCst);

            let trigger_text = w!("✨ Trigger MagicCursor (Alt+G)");
            let pause_text = if is_paused {
                w!("▶ Resume Triggers")
            } else {
                w!("⏸ Pause Triggers")
            };
            let config_text = w!("⚙ Open config.toml");
            let folder_text = w!("📁 Open Config Folder");
            let exit_text = w!("❌ Exit MagicCursor");

            let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_TRIGGER, trigger_text);
            let pause_flags = if is_paused { MF_CHECKED } else { MF_UNCHECKED } | MF_STRING;
            let _ = AppendMenuW(menu, pause_flags, ID_TRAY_TOGGLE_PAUSE, pause_text);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_OPEN_CONFIG, config_text);
            let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_OPEN_FOLDER, folder_text);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_EXIT, exit_text);

            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(
                menu,
                TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                None,
                hwnd,
                None,
            );

            let _ = DestroyMenu(menu);
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
        }
    }
}
