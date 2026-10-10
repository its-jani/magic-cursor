use arboard::Clipboard;
use base64::Engine;
use image::{ImageBuffer, Rgba};
use std::io::Cursor;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::screen::{capture_screen_region, CapturedImage};
use crate::config::Config;

#[derive(Debug, Clone)]
pub enum CapturedContext {
    Empty,
    Text(String),
    Image(CapturedImage),
}

impl CapturedContext {
    pub fn is_empty(&self) -> bool {
        matches!(self, CapturedContext::Empty)
    }

    pub fn summary(&self) -> String {
        match self {
            CapturedContext::Empty => "No context attached".to_string(),
            CapturedContext::Text(t) => {
                let char_count = t.chars().count();
                let line_count = t.lines().count();
                format!("Text ({} chars, {} lines)", char_count, line_count)
            }
            CapturedContext::Image(img) => {
                format!("Screen Region ({}x{} px)", img.width, img.height)
            }
        }
    }
}

pub fn get_current_cursor_pos() -> (i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        (pt.x, pt.y)
    }
}

/// Inspects the system clipboard for text or image; if empty, falls back to screen crop.
pub fn capture_active_context(config: &Config, cursor_x: i32, cursor_y: i32) -> CapturedContext {
    // 1. Try reading clipboard text
    if let Ok(mut clipboard) = Clipboard::new() {
        if let Ok(text) = clipboard.get_text() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return CapturedContext::Text(trimmed.to_string());
            }
        }

        // 2. Try reading clipboard image
        if let Ok(img_data) = clipboard.get_image() {
            let w = img_data.width as u32;
            let h = img_data.height as u32;
            let rgba_raw = img_data.bytes.into_owned();

            if let Some(img) = ImageBuffer::<Rgba<u8>, _>::from_raw(w, h, rgba_raw.clone()) {
                let mut png_bytes = Vec::new();
                let mut cursor = Cursor::new(&mut png_bytes);
                if img.write_to(&mut cursor, image::ImageFormat::Png).is_ok() {
                    let base64_png = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
                    return CapturedContext::Image(CapturedImage {
                        base64_png,
                        width: w,
                        height: h,
                        raw_rgba: rgba_raw,
                    });
                }
            }
        }
    }

    // 3. Fallback: Localized screen capture around cursor
    if config.overlay.auto_capture_screen {
        if let Some(screen_crop) = capture_screen_region(
            cursor_x,
            cursor_y,
            config.overlay.crop_width,
            config.overlay.crop_height,
        ) {
            return CapturedContext::Image(screen_crop);
        }
    }

    CapturedContext::Empty
}
