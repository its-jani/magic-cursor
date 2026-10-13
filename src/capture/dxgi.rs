use base64::Engine;
use image::{ImageBuffer, Rgba};
use std::io::Cursor;
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIOutputDuplication, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub raw_rgba: Vec<u8>,
}

impl CapturedFrame {
    pub fn new(x: i32, y: i32, width: u32, height: u32, raw_rgba: Vec<u8>) -> Self {
        Self {
            x,
            y,
            width,
            height,
            raw_rgba,
        }
    }

    /// Crops a sub-region `[crop_x, crop_y, crop_w, crop_h]` from this frame in desktop coordinates.
    pub fn crop(&self, crop_x: i32, crop_y: i32, crop_w: u32, crop_h: u32) -> Option<Self> {
        if crop_w == 0 || crop_h == 0 {
            return None;
        }

        let rel_x = crop_x - self.x;
        let rel_y = crop_y - self.y;

        if rel_x < 0 || rel_y < 0 {
            return None;
        }

        let rel_x = rel_x as u32;
        let rel_y = rel_y as u32;

        if rel_x + crop_w > self.width || rel_y + crop_h > self.height {
            return None;
        }

        let mut cropped_rgba = Vec::with_capacity((crop_w * crop_h * 4) as usize);

        for row in 0..crop_h {
            let src_row = rel_y + row;
            let src_start = ((src_row * self.width + rel_x) * 4) as usize;
            let src_end = src_start + (crop_w * 4) as usize;
            cropped_rgba.extend_from_slice(&self.raw_rgba[src_start..src_end]);
        }

        Some(Self {
            x: crop_x,
            y: crop_y,
            width: crop_w,
            height: crop_h,
            raw_rgba: cropped_rgba,
        })
    }

    /// Encodes the frame to a Base64-encoded PNG string for multi-modal Vision LLM inputs.
    pub fn to_base64_png(&self) -> Option<String> {
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(self.width, self.height, self.raw_rgba.clone())?;

        let mut png_bytes = Vec::new();
        let mut cursor = Cursor::new(&mut png_bytes);
        img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;

        Some(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
    }
}

// =========================================================================
// High-Performance DXGI Desktop Duplication Engine
// =========================================================================

pub struct DxgiCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: Option<IDXGIOutputDuplication>,
    output_desc_rect: RECT,
}

impl DxgiCapturer {
    pub fn new() -> Option<Self> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];

            let hr = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                windows::Win32::Foundation::HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            );

            let (device, context) = match (hr, device, context) {
                (Ok(_), Some(d), Some(c)) => (d, c),
                _ => {
                    // Fallback to WARP driver
                    let mut warp_dev = None;
                    let mut warp_ctx = None;
                    let warp_hr = D3D11CreateDevice(
                        None,
                        D3D_DRIVER_TYPE_WARP,
                        windows::Win32::Foundation::HMODULE::default(),
                        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                        Some(&feature_levels),
                        D3D11_SDK_VERSION,
                        Some(&mut warp_dev),
                        None,
                        Some(&mut warp_ctx),
                    );
                    match (warp_hr, warp_dev, warp_ctx) {
                        (Ok(_), Some(d), Some(c)) => (d, c),
                        _ => return None,
                    }
                }
            };

            let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            let adapter: IDXGIAdapter1 = factory.EnumAdapters1(0).ok()?;
            let output: IDXGIOutput = adapter.EnumOutputs(0).ok()?;
            let output_desc = output.GetDesc().ok()?;
            let output1: IDXGIOutput1 = output.cast().ok()?;

            let duplication = output1.DuplicateOutput(&device).ok();

            Some(Self {
                device,
                context,
                duplication,
                output_desc_rect: output_desc.DesktopCoordinates,
            })
        }
    }

    pub fn capture_frame(&mut self) -> Option<CapturedFrame> {
        let dupl = self.duplication.as_ref()?;

        unsafe {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut desktop_resource: Option<windows::Win32::Graphics::Dxgi::IDXGIResource> = None;

            // Try acquiring frame with 50ms timeout
            let acquire_res = dupl.AcquireNextFrame(50, &mut frame_info, &mut desktop_resource);
            if acquire_res.is_err() {
                // If frame wasn't updated or timeout, retry re-initializing
                return None;
            }

            let resource = desktop_resource?;
            let texture: ID3D11Texture2D = resource.cast().ok()?;

            let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut tex_desc);

            let width = tex_desc.Width;
            let height = tex_desc.Height;

            // Create staging texture with CPU read access
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: tex_desc.Format,
                SampleDesc: tex_desc.SampleDesc,
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let mut staging_texture: Option<ID3D11Texture2D> = None;
            if self
                .device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging_texture))
                .is_err()
            {
                let _ = dupl.ReleaseFrame();
                return None;
            }

            let staging = staging_texture?;

            let src_res: ID3D11Resource = texture.cast().ok()?;
            let dst_res: ID3D11Resource = staging.cast().ok()?;
            self.context.CopyResource(&dst_res, &src_res);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            let map_res = self
                .context
                .Map(&dst_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped));

            if map_res.is_err() {
                let _ = dupl.ReleaseFrame();
                return None;
            }

            let row_pitch = mapped.RowPitch as usize;
            let p_data = mapped.pData as *const u8;

            let mut rgba_buffer = vec![0u8; (width * height * 4) as usize];

            let is_bgra = tex_desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM;

            for y in 0..(height as usize) {
                let src_row = std::slice::from_raw_parts(p_data.add(y * row_pitch), (width * 4) as usize);
                let dst_offset = y * (width as usize) * 4;

                if is_bgra {
                    for x in 0..(width as usize) {
                        let si = x * 4;
                        let di = dst_offset + x * 4;
                        rgba_buffer[di] = src_row[si + 2];     // R
                        rgba_buffer[di + 1] = src_row[si + 1]; // G
                        rgba_buffer[di + 2] = src_row[si];     // B
                        rgba_buffer[di + 3] = 255;             // A
                    }
                } else {
                    rgba_buffer[dst_offset..dst_offset + (width as usize * 4)]
                        .copy_from_slice(&src_row[..(width as usize * 4)]);
                }
            }

            self.context.Unmap(&dst_res, 0);
            let _ = dupl.ReleaseFrame();

            Some(CapturedFrame {
                x: self.output_desc_rect.left,
                y: self.output_desc_rect.top,
                width,
                height,
                raw_rgba: rgba_buffer,
            })
        }
    }
}

// =========================================================================
// Robust Win32 GDI Fallback Screen Capture Engine
// =========================================================================

pub fn capture_screen_gdi(
    crop_x: i32,
    crop_y: i32,
    target_w: u32,
    target_h: u32,
) -> Option<CapturedFrame> {
    unsafe {
        let hdc_screen: HDC = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }

        let hdc_mem: HDC = CreateCompatibleDC(Some(hdc_screen));
        if hdc_mem.is_invalid() {
            let _ = ReleaseDC(None, hdc_screen);
            return None;
        }

        let hbitmap: HBITMAP = CreateCompatibleBitmap(hdc_screen, target_w as i32, target_h as i32);
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
            target_w as i32,
            target_h as i32,
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

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: target_w as i32,
                biHeight: -(target_h as i32), // Top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (target_w * target_h * 4),
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
            target_h,
            Some(bgra_buffer.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

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
            rgba_buffer[i] = bgra_buffer[i + 2];     // R
            rgba_buffer[i + 1] = bgra_buffer[i + 1]; // G
            rgba_buffer[i + 2] = bgra_buffer[i];     // B
            rgba_buffer[i + 3] = 255;                // A
        }

        Some(CapturedFrame {
            x: crop_x,
            y: crop_y,
            width: target_w,
            height: target_h,
            raw_rgba: rgba_buffer,
        })
    }
}

/// Captures the full virtual screen (multi-monitor capable) using DXGI with seamless GDI fallback.
pub fn capture_full_screen() -> Option<CapturedFrame> {
    // 1. Try DXGI Desktop Duplication for sub-10ms capture
    if let Some(mut capturer) = DxgiCapturer::new() {
        if let Some(frame) = capturer.capture_frame() {
            return Some(frame);
        }
    }

    // 2. Fallback to GDI Virtual Desktop capture
    unsafe {
        let virt_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let virt_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let virt_w = GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32;
        let virt_h = GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32;

        capture_screen_gdi(virt_x, virt_y, virt_w, virt_h)
    }
}

/// Captures a localized rectangular region around (center_x, center_y).
pub fn capture_region(center_x: i32, center_y: i32, width: u32, height: u32) -> Option<CapturedFrame> {
    unsafe {
        let virt_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let virt_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let virt_w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let virt_h = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let target_w = width.clamp(50, virt_w as u32);
        let target_h = height.clamp(50, virt_h as u32);

        let mut crop_x = center_x - (target_w as i32 / 2);
        let mut crop_y = center_y - (target_h as i32 / 2);

        if crop_x < virt_x {
            crop_x = virt_x;
        }
        if crop_y < virt_y {
            crop_y = virt_y;
        }
        if crop_x + target_w as i32 > virt_x + virt_w {
            crop_x = (virt_x + virt_w) - target_w as i32;
        }
        if crop_y + target_h as i32 > virt_y + virt_h {
            crop_y = (virt_y + virt_h) - target_h as i32;
        }

        // Fast GDI region capture
        capture_screen_gdi(crop_x, crop_y, target_w, target_h)
    }
}
