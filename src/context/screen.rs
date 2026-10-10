use base64::Engine;
use image::{ImageBuffer, Rgba};
use std::io::Cursor;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub base64_png: String,
    pub width: u32,
    pub height: u32,
    pub raw_rgba: Vec<u8>,
}

/// Captures a localized rectangular screen region centered at (cursor_x, cursor_y).
pub fn capture_screen_region(
    cursor_x: i32,
    cursor_y: i32,
    width: i32,
    height: i32,
) -> Option<CapturedImage> {
    unsafe {
        // Virtual screen bounds (multi-monitor support)
        let virt_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let virt_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let virt_w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let virt_h = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let target_w = width.clamp(100, 1920);
        let target_h = height.clamp(100, 1080);

        // Center on cursor, clamped within virtual desktop
        let mut crop_x = cursor_x - (target_w / 2);
        let mut crop_y = cursor_y - (target_h / 2);

        if crop_x < virt_x {
            crop_x = virt_x;
        }
        if crop_y < virt_y {
            crop_y = virt_y;
        }
        if crop_x + target_w > virt_x + virt_w {
            crop_x = (virt_x + virt_w) - target_w;
        }
        if crop_y + target_h > virt_y + virt_h {
            crop_y = (virt_y + virt_h) - target_h;
        }

        // Win32 GDI capture
        let hdc_screen: HDC = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }

        let hdc_mem: HDC = CreateCompatibleDC(Some(hdc_screen));
        if hdc_mem.is_invalid() {
            let _ = ReleaseDC(None, hdc_screen);
            return None;
        }

        let hbitmap: HBITMAP = CreateCompatibleBitmap(hdc_screen, target_w, target_h);
        if hbitmap.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(None, hdc_screen);
            return None;
        }

        let old_obj = SelectObject(hdc_mem, hbitmap.into());

        let blt_res = BitBlt(
            hdc_mem,
            0,
            0,
            target_w,
            target_h,
            Some(hdc_screen),
            crop_x,
            crop_y,
            SRCCOPY,
        );

        if blt_res.is_err() {
            SelectObject(hdc_mem, old_obj);
            let _ = DeleteObject(hbitmap.into());
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(None, hdc_screen);
            return None;
        }

        // Extract raw bitmap bits
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: target_w,
                biHeight: -target_h, // Top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (target_w * target_h * 4) as u32,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD::default(); 1],
        };

        let mut bgra_buffer: Vec<u8> = vec![0u8; (target_w * target_h * 4) as usize];

        let scan_lines = GetDIBits(
            hdc_mem,
            hbitmap,
            0,
            target_h as u32,
            Some(bgra_buffer.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // Cleanup GDI objects immediately
        SelectObject(hdc_mem, old_obj);
        let _ = DeleteObject(hbitmap.into());
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(None, hdc_screen);

        if scan_lines == 0 {
            return None;
        }

        // Convert BGRA to RGBA
        let mut rgba_buffer: Vec<u8> = vec![0u8; bgra_buffer.len()];
        for i in (0..bgra_buffer.len()).step_by(4) {
            let b = bgra_buffer[i];
            let g = bgra_buffer[i + 1];
            let r = bgra_buffer[i + 2];
            let a = 255u8;
            rgba_buffer[i] = r;
            rgba_buffer[i + 1] = g;
            rgba_buffer[i + 2] = b;
            rgba_buffer[i + 3] = a;
        }

        // Encode to PNG in memory
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(target_w as u32, target_h as u32, rgba_buffer.clone())?;

        let mut png_bytes = Vec::new();
        let mut cursor = Cursor::new(&mut png_bytes);
        if img.write_to(&mut cursor, image::ImageFormat::Png).is_err() {
            return None;
        }

        let base64_png = base64::engine::general_purpose::STANDARD.encode(&png_bytes);

        Some(CapturedImage {
            base64_png,
            width: target_w as u32,
            height: target_h as u32,
            raw_rgba: rgba_buffer,
        })
    }
}
