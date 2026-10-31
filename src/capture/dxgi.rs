//! Desktop frame capture through DXGI Desktop Duplication, with a GDI fallback.
//!
//! Duplication reads the frame on the GPU and copies it into a staging texture,
//! which is what keeps a full-screen grab under ten milliseconds. GDI `BitBlt`
//! is slower but works in the sessions where duplication refuses to start.

use std::fmt;
use std::io::Cursor;

use base64::Engine;
use image::{ImageBuffer, Rgba};
use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// Bytes per pixel in every buffer this module produces.
const BYTES_PER_PIXEL: usize = 4;

/// Milliseconds `AcquireNextFrame` waits for a new desktop frame.
const ACQUIRE_TIMEOUT_MS: u32 = 50;

/// Smallest capture region accepted by [`capture_region`].
const MIN_REGION_PX: u32 = 50;

/// A captured desktop frame in row-major RGBA8 order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    /// Left edge in virtual-screen coordinates.
    pub x: i32,
    /// Top edge in virtual-screen coordinates.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA data.
    pub raw_rgba: Vec<u8>,
}

impl CapturedFrame {
    /// Creates a frame from raw RGBA pixels.
    pub fn new(x: i32, y: i32, width: u32, height: u32, raw_rgba: Vec<u8>) -> Self {
        Self {
            x,
            y,
            width,
            height,
            raw_rgba,
        }
    }

    /// Crops a sub-region given in virtual-screen coordinates.
    ///
    /// Returns `None` when the region is empty, starts outside the frame, or
    /// extends past its bottom-right corner, and never panics on a bad rectangle.
    pub fn crop(&self, crop_x: i32, crop_y: i32, crop_w: u32, crop_h: u32) -> Option<Self> {
        if crop_w == 0 || crop_h == 0 {
            return None;
        }

        let rel_x = u32::try_from(crop_x.checked_sub(self.x)?).ok()?;
        let rel_y = u32::try_from(crop_y.checked_sub(self.y)?).ok()?;

        if rel_x.checked_add(crop_w)? > self.width || rel_y.checked_add(crop_h)? > self.height {
            return None;
        }

        let row_bytes = (crop_w as usize) * BYTES_PER_PIXEL;
        let mut raw_rgba = Vec::with_capacity(row_bytes * (crop_h as usize));

        for row in rel_y..(rel_y + crop_h) {
            let start = (row as usize * self.width as usize + rel_x as usize) * BYTES_PER_PIXEL;
            let row_slice = self.raw_rgba.get(start..start.checked_add(row_bytes)?)?;
            raw_rgba.extend_from_slice(row_slice);
        }

        Some(Self {
            x: crop_x,
            y: crop_y,
            width: crop_w,
            height: crop_h,
            raw_rgba,
        })
    }

    /// Encodes the frame as a base64 PNG for vision models, consuming it.
    ///
    /// Returns `None` when the pixel buffer does not match the frame geometry or
    /// PNG encoding fails.
    pub fn into_base64_png(self) -> Option<String> {
        let image: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(self.width, self.height, self.raw_rgba)?;

        let mut png_bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .ok()?;

        Some(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
    }
}

impl VirtualScreen {
    /// Reads the bounds of the virtual desktop from the system metrics.
    pub fn current() -> Self {
        // SAFETY: GetSystemMetrics takes an index and returns a plain integer; the
        // four indices below are documented virtual-screen metrics.
        unsafe {
            Self {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                width: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(0) as u32,
                height: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(0) as u32,
            }
        }
    }
}

/// Bounds of the virtual desktop spanning every monitor, in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualScreen {
    /// Left edge of the left-most monitor.
    pub x: i32,
    /// Top edge of the top-most monitor.
    pub y: i32,
    /// Combined width in pixels.
    pub width: u32,
    /// Combined height in pixels.
    pub height: u32,
}

/// Owns a D3D11 device, its immediate context, and the desktop duplication.
pub struct DxgiCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: Option<IDXGIOutputDuplication>,
    output_rect: RECT,
}

impl fmt::Debug for DxgiCapturer {
    /// Reports whether a duplication is active without dumping COM pointers.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DxgiCapturer")
            .field("duplicating", &self.duplication.is_some())
            .field("output_rect", &self.output_rect)
            .finish_non_exhaustive()
    }
}

impl DxgiCapturer {
    /// Creates a hardware device, falling back to WARP when hardware is refused.
    ///
    /// Returns `None` when neither driver can be created or no output can be
    /// duplicated, in which case the caller should use the GDI fallback.
    pub fn new() -> Option<Self> {
        let (device, context) = create_device(D3D_DRIVER_TYPE_HARDWARE)
            .or_else(|| create_device(D3D_DRIVER_TYPE_WARP))?;

        // SAFETY: both calls take no Rust-owned pointers and only return COM
        // interfaces that the runtime keeps alive through reference counting.
        let (output, output_rect) = unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            let adapter = factory.EnumAdapters1(0).ok()?;
            let output = adapter.EnumOutputs(0).ok()?;
            let rect = output.GetDesc().ok()?.DesktopCoordinates;
            (output, rect)
        };

        // SAFETY: `output.cast` only queries a different interface on the same COM
        // object, and `DuplicateOutput` hands the frame buffer to this device.
        let duplication = unsafe {
            let output1: IDXGIOutput1 = output.cast().ok()?;
            output1.DuplicateOutput(&device).ok()
        };

        Some(Self {
            device,
            context,
            duplication,
            output_rect,
        })
    }

    /// Captures the next desktop frame.
    ///
    /// Returns `None` when no new frame arrived within [`ACQUIRE_TIMEOUT_MS`],
    /// when the duplication was never created, or when the read-back failed.
    pub fn capture_frame(&mut self) -> Option<CapturedFrame> {
        let frame = self.acquire()?;
        let texture = frame.texture.as_ref()?;

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `desc` is a stack local owned by this scope.
        unsafe { texture.GetDesc(&mut desc) };

        let (width, height) = (desc.Width, desc.Height);
        if width == 0 || height == 0 {
            return None;
        }

        let staging = self.create_staging_texture(&desc)?;
        let is_bgra = desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM;
        let raw_rgba = self.read_back(texture, &staging, width, height, is_bgra)?;

        Some(CapturedFrame {
            x: self.output_rect.left,
            y: self.output_rect.top,
            width,
            height,
            raw_rgba,
        })
    }

    /// Acquires the next frame, keeping it released through [`DuplicatedFrame`].
    fn acquire(&self) -> Option<DuplicatedFrame<'_>> {
        let duplication = self.duplication.as_ref()?;
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        // SAFETY: `duplication` is a live interface owned by `self`, and both
        // out-parameters are stack locals that outlive the call.
        unsafe {
            duplication
                .AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut frame_info, &mut resource)
                .ok()?;
        }

        // The frame belongs to us from here on, so the guard must exist before the
        // remaining conversions can fail.
        let texture = resource
            .as_ref()
            .and_then(|resource| resource.cast::<ID3D11Texture2D>().ok());

        Some(DuplicatedFrame {
            duplication,
            texture,
        })
    }

    /// Creates a CPU-readable copy target matching `source_desc`.
    fn create_staging_texture(
        &self,
        source_desc: &D3D11_TEXTURE2D_DESC,
    ) -> Option<ID3D11Texture2D> {
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: source_desc.Width,
            Height: source_desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: source_desc.Format,
            SampleDesc: source_desc.SampleDesc,
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };

        let mut staging: Option<ID3D11Texture2D> = None;
        // SAFETY: the descriptor is fully initialised above and the out-parameter is
        // a stack local.
        unsafe {
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .ok()?;
        };

        staging
    }

    /// Copies the frame to staging memory and converts it to RGBA.
    fn read_back(
        &self,
        texture: &ID3D11Texture2D,
        staging: &ID3D11Texture2D,
        width: u32,
        height: u32,
        is_bgra: bool,
    ) -> Option<Vec<u8>> {
        // SAFETY: both textures were created by (or for) `self.device` and have the
        // same description, which is what CopyResource requires.
        unsafe {
            let source = texture_to_resource(texture)?;
            let target = texture_to_resource(staging)?;
            self.context.CopyResource(&target, &source);
        }

        let target: ID3D11Resource = texture_to_resource(staging)?;

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: mapping a staging resource read-only is allowed here because the
        // GPU copy above has completed on the immediate context.
        unsafe {
            self.context
                .Map(&target, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .ok()?;
        };

        let guard = MappedResource {
            context: &self.context,
            resource: &target,
        };

        let row_pitch = mapped.RowPitch as usize;
        let mut raw_rgba = vec![0u8; (width as usize) * (height as usize) * BYTES_PER_PIXEL];

        for row in 0..(height as usize) {
            // SAFETY: the mapped buffer holds `RowPitch * Height` readable bytes, and
            // `width * 4` never exceeds `RowPitch` for a B8G8R8A8 texture.
            let source_row = unsafe {
                std::slice::from_raw_parts(
                    mapped.pData.add(row * row_pitch).cast::<u8>(),
                    width as usize * BYTES_PER_PIXEL,
                )
            };

            let destination_row = &mut raw_rgba[row * width as usize * BYTES_PER_PIXEL..]
                [..width as usize * BYTES_PER_PIXEL];

            if is_bgra {
                bgra_to_rgba_row(source_row, destination_row);
            } else {
                destination_row.copy_from_slice(source_row);
            }
        }

        // Unmaps before the buffer is used, so the guard's job is done.
        drop(guard);

        Some(raw_rgba)
    }
}

/// A frame acquired from the duplication that releases itself on drop.
struct DuplicatedFrame<'a> {
    duplication: &'a IDXGIOutputDuplication,
    texture: Option<ID3D11Texture2D>,
}

impl Drop for DuplicatedFrame<'_> {
    fn drop(&mut self) {
        // SAFETY: `AcquireNextFrame` succeeded before this guard was built, and the
        // duplication outlives it, so exactly one release happens here.
        unsafe {
            let _ = self.duplication.ReleaseFrame();
        }
    }
}

/// A mapped staging resource that unmaps itself on drop.
struct MappedResource<'a> {
    context: &'a ID3D11DeviceContext,
    resource: &'a ID3D11Resource,
}

impl Drop for MappedResource<'_> {
    fn drop(&mut self) {
        // SAFETY: the resource was mapped by the same context and is unmapped once.
        unsafe { self.context.Unmap(self.resource, 0) };
    }
}

/// Creates a D3D11 device and its immediate context for the given driver.
fn create_device(driver: D3D_DRIVER_TYPE) -> Option<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let feature_levels = [D3D_FEATURE_LEVEL_11_0];

    // SAFETY: all pointers are either null (adapter) or stack locals that outlive
    // the call, and `feature_levels` lives for the duration of the call.
    let created = unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };

    created.ok()?;
    Some((device?, context?))
}

/// Casts a texture to the generic resource interface used by copy and map.
fn texture_to_resource(texture: &ID3D11Texture2D) -> Option<ID3D11Resource> {
    // Every texture also implements ID3D11Resource, so this is a plain interface
    // query on a live COM object.
    texture.cast().ok()
}

/// Converts one BGRA row to RGBA, forcing the alpha channel opaque.
fn bgra_to_rgba_row(bgra: &[u8], rgba: &mut [u8]) {
    for (destination, source) in rgba
        .chunks_exact_mut(BYTES_PER_PIXEL)
        .zip(bgra.chunks_exact(BYTES_PER_PIXEL))
    {
        destination[0] = source[2];
        destination[1] = source[1];
        destination[2] = source[0];
        destination[3] = u8::MAX;
    }
}

/// Converts a whole BGRA buffer to RGBA, forcing the alpha channel opaque.
fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = vec![0u8; bgra.len()];
    bgra_to_rgba_row(bgra, &mut rgba);
    rgba
}

/// A screen DC released on drop.
struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: the DC came from GetDC(None) and is released exactly once.
        unsafe {
            let _ = ReleaseDC(None, self.0);
        }
    }
}

/// A memory DC deleted on drop.
struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: the DC came from CreateCompatibleDC and is deleted exactly once.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// A bitmap deleted on drop.
struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateCompatibleBitmap and is owned here.
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

/// Captures a rectangle of the desktop with `BitBlt` into a 32-bit top-down DIB.
///
/// Returns `None` when any GDI call in the chain fails.
pub fn capture_screen_gdi(
    crop_x: i32,
    crop_y: i32,
    target_w: u32,
    target_h: u32,
) -> Option<CapturedFrame> {
    if target_w == 0 || target_h == 0 {
        return None;
    }

    // SAFETY: GetDC(None) hands back a DC for the whole virtual screen; the wrapper
    // releases it on every path, including the early returns below.
    let screen = ScreenDc(unsafe { GetDC(None) });
    if screen.0.is_invalid() {
        return None;
    }

    // SAFETY: a compatible DC and bitmap of the requested size are valid operands
    // for BitBlt; both wrappers free their handle on drop.
    let (memory, bitmap, previous) = unsafe {
        let memory = MemoryDc(CreateCompatibleDC(Some(screen.0)));
        if memory.0.is_invalid() {
            return None;
        }

        let bitmap = Bitmap(CreateCompatibleBitmap(
            screen.0,
            target_w as i32,
            target_h as i32,
        ));
        if bitmap.0.is_invalid() {
            return None;
        }

        let previous = SelectObject(memory.0, bitmap.0.into());
        (memory, bitmap, previous)
    };

    // SAFETY: `memory` is a valid memory DC with `bitmap` selected into it.
    let blitted = unsafe {
        BitBlt(
            memory.0,
            0,
            0,
            target_w as i32,
            target_h as i32,
            Some(screen.0),
            crop_x,
            crop_y,
            SRCCOPY,
        )
    };

    // Restore the original object before the bitmap is deleted, whatever happened.
    // SAFETY: `previous` was returned by SelectObject on this same DC.
    unsafe {
        SelectObject(memory.0, previous);
    }

    if blitted.is_err() {
        drop(bitmap);
        return None;
    }

    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: target_w as i32,
            // Negative height requests top-down rows, matching RGBA layout.
            biHeight: -(target_h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: target_w * target_h * BYTES_PER_PIXEL as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..BITMAPINFO::default()
    };

    let mut bgra = vec![0u8; (target_w as usize) * (target_h as usize) * BYTES_PER_PIXEL];

    // SAFETY: `bgra` is exactly the size described by `info`, and both outlive the call.
    let scan_lines = unsafe {
        GetDIBits(
            memory.0,
            bitmap.0,
            0,
            target_h,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        )
    };

    if scan_lines == 0 {
        return None;
    }

    Some(CapturedFrame {
        x: crop_x,
        y: crop_y,
        width: target_w,
        height: target_h,
        raw_rgba: bgra_to_rgba(&bgra),
    })
}

/// Captures the whole virtual screen, preferring desktop duplication.
pub fn capture_full_screen() -> Option<CapturedFrame> {
    if let Some(frame) = DxgiCapturer::new().and_then(|mut capturer| capturer.capture_frame()) {
        return Some(frame);
    }

    let screen = VirtualScreen::current();
    capture_screen_gdi(screen.x, screen.y, screen.width, screen.height)
}

/// Captures a region centered on `(center_x, center_y)`, clamped to the desktop.
pub fn capture_region(
    center_x: i32,
    center_y: i32,
    width: u32,
    height: u32,
) -> Option<CapturedFrame> {
    let screen = VirtualScreen::current();
    if screen.width == 0 || screen.height == 0 {
        return None;
    }

    let target_w = width.clamp(MIN_REGION_PX, screen.width);
    let target_h = height.clamp(MIN_REGION_PX, screen.height);

    let max_x = screen.x + (screen.width - target_w) as i32;
    let max_y = screen.y + (screen.height - target_h) as i32;

    let crop_x = (center_x - target_w as i32 / 2).clamp(screen.x, max_x);
    let crop_y = (center_y - target_h as i32 / 2).clamp(screen.y, max_y);

    capture_screen_gdi(crop_x, crop_y, target_w, target_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a frame whose red channel encodes its column and green its row.
    fn gradient_frame(width: u32, height: u32) -> CapturedFrame {
        let mut raw_rgba = Vec::with_capacity((width * height) as usize * BYTES_PER_PIXEL);
        for row in 0..height {
            for column in 0..width {
                raw_rgba.extend_from_slice(&[column as u8, row as u8, 0, 255]);
            }
        }
        CapturedFrame::new(-100, -50, width, height, raw_rgba)
    }

    #[test]
    fn crop_copies_the_requested_rows() {
        let frame = gradient_frame(8, 6);
        let crop = frame
            .crop(-98, -48, 3, 2)
            .expect("region is inside the frame");

        assert_eq!((crop.width, crop.height), (3, 2));
        assert_eq!((crop.x, crop.y), (-98, -48));
        assert_eq!(
            crop.raw_rgba,
            vec![
                2, 2, 0, 255, 3, 2, 0, 255, 4, 2, 0, 255, 2, 3, 0, 255, 3, 3, 0, 255, 4, 3, 0, 255
            ]
        );
    }

    #[test]
    fn crop_rejects_regions_outside_the_frame() {
        let frame = gradient_frame(8, 6);

        assert!(frame.crop(-100, -50, 0, 4).is_none(), "zero width");
        assert!(
            frame.crop(-101, -50, 2, 2).is_none(),
            "starts left of the frame"
        );
        assert!(
            frame.crop(-100, -50, 9, 2).is_none(),
            "wider than the frame"
        );
        assert!(
            frame.crop(-100, -50, 8, 7).is_none(),
            "taller than the frame"
        );
    }

    #[test]
    fn crop_of_the_full_frame_round_trips() {
        let frame = gradient_frame(4, 4);
        let crop = frame
            .crop(frame.x, frame.y, frame.width, frame.height)
            .expect("full frame");

        assert_eq!(crop.raw_rgba, frame.raw_rgba);
    }

    #[test]
    fn png_encoding_requires_matching_geometry() {
        let frame = CapturedFrame::new(0, 0, 4, 4, vec![0; 8]);

        assert!(
            frame.into_base64_png().is_none(),
            "short buffer must not encode"
        );
    }

    #[test]
    fn png_encoding_produces_base64() {
        let encoded = gradient_frame(2, 2).into_base64_png().expect("encodes");

        assert!(encoded.starts_with("iVBORw0KGgo"), "got {encoded}");
    }

    #[test]
    fn bgra_rows_swap_red_and_blue() {
        let mut rgba = [0u8; 4];

        bgra_to_rgba_row(&[1, 2, 3, 4], &mut rgba);

        assert_eq!(rgba, [3, 2, 1, 255]);
    }
}
