use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN, VK_ESCAPE, VK_F1, VK_F12, VK_RETURN, VK_SPACE, VK_TAB,
};

pub struct HotkeyDefinition {
    pub modifiers: u32,
    pub vk_code: u32,
}

impl HotkeyDefinition {
    /// Parses a hotkey string such as "Alt+G", "Win+Alt+Space", "Ctrl+Shift+F1"
    pub fn parse(s: &str) -> Option<Self> {
        let mut modifiers = MOD_NOREPEAT.0;
        let mut vk_code = 0u32;

        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
        if parts.is_empty() {
            return None;
        }

        for (idx, part) in parts.iter().enumerate() {
            let is_last = idx == parts.len() - 1;
            let upper = part.to_uppercase();

            match upper.as_str() {
                "ALT" | "OPTION" => modifiers |= MOD_ALT.0,
                "CTRL" | "CONTROL" => modifiers |= MOD_CONTROL.0,
                "SHIFT" => modifiers |= MOD_SHIFT.0,
                "WIN" | "WINDOWS" | "SUPER" | "CMD" => modifiers |= MOD_WIN.0,
                _ if is_last => {
                    vk_code = parse_vk_code(&upper)?;
                }
                _ => return None,
            }
        }

        if vk_code == 0 {
            return None;
        }

        Some(Self { modifiers, vk_code })
    }
}

fn parse_vk_code(key: &str) -> Option<u32> {
    if key.len() == 1 {
        let ch = key.chars().next().unwrap();
        if ch.is_ascii_alphanumeric() {
            return Some(ch.to_ascii_uppercase() as u32);
        }
    }

    match key {
        "SPACE" => Some(VK_SPACE.0 as u32),
        "ENTER" | "RETURN" => Some(VK_RETURN.0 as u32),
        "TAB" => Some(VK_TAB.0 as u32),
        "ESC" | "ESCAPE" => Some(VK_ESCAPE.0 as u32),
        "F1" => Some(VK_F1.0 as u32),
        "F2" => Some((VK_F1.0 + 1) as u32),
        "F3" => Some((VK_F1.0 + 2) as u32),
        "F4" => Some((VK_F1.0 + 3) as u32),
        "F5" => Some((VK_F1.0 + 4) as u32),
        "F6" => Some((VK_F1.0 + 5) as u32),
        "F7" => Some((VK_F1.0 + 6) as u32),
        "F8" => Some((VK_F1.0 + 7) as u32),
        "F9" => Some((VK_F1.0 + 8) as u32),
        "F10" => Some((VK_F1.0 + 9) as u32),
        "F11" => Some((VK_F1.0 + 10) as u32),
        "F12" => Some(VK_F12.0 as u32),
        _ => None,
    }
}

pub struct HotkeyManager {
    hwnd: HWND,
    hotkey_id: i32,
    is_registered: bool,
}

impl HotkeyManager {
    pub fn new(hwnd: HWND, hotkey_id: i32) -> Self {
        Self {
            hwnd,
            hotkey_id,
            is_registered: false,
        }
    }

    pub fn register(&mut self, definition: &HotkeyDefinition) -> bool {
        self.unregister();
        unsafe {
            let res = RegisterHotKey(
                Some(self.hwnd),
                self.hotkey_id,
                HOT_KEY_MODIFIERS(definition.modifiers),
                definition.vk_code,
            );
            self.is_registered = res.is_ok();
            self.is_registered
        }
    }

    pub fn unregister(&mut self) {
        if self.is_registered {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), self.hotkey_id);
            }
            self.is_registered = false;
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister();
    }
}
