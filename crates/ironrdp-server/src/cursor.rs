//! Windows cursor shape capture for server display backends.

use std::io;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW, HBITMAP, HDC, HGDIOBJ, RGBQUAD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, GetCursorInfo, GetIconInfo, HCURSOR, HICON, ICONINFO,
};

use crate::{ColorPointer, DisplayUpdate, RGBAPointer};

const MAX_DIMENSION: u16 = 384;

/// Polls cursor visibility and shape on the calling thread's Windows desktop.
///
/// Create a new capture for each client activation so the initial shape is sent again.
/// Cursor positions and input-origin tracking remain the caller's responsibility.
#[derive(Default)]
pub struct CursorCapture {
    // None means no sample yet; Some(None) means the last sample was hidden.
    previous: Option<Option<CursorImage>>,
}

impl CursorCapture {
    /// Returns a shape or visibility update, or `None` when neither changed.
    ///
    /// Images larger than 384 pixels in either dimension and invalid hotspots return an error.
    /// Errors invalidate the cached state, so the next successful capture resends visibility or shape.
    /// The caller decides whether to hide the pointer while capture is unavailable.
    /// Updates use cache index zero; the server encoder applies negotiated pointer limits.
    pub fn capture(&mut self) -> io::Result<Option<DisplayUpdate>> {
        self.update(capture_image())
    }

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    fn update(&mut self, image: io::Result<Option<CursorImage>>) -> io::Result<Option<DisplayUpdate>> {
        let image = image.inspect_err(|_| self.previous = None)?;
        if self.previous.as_ref() == Some(&image) {
            return Ok(None);
        }
        let update = image.as_ref().map_or(DisplayUpdate::HidePointer, CursorImage::update);
        self.previous = Some(image);
        Ok(Some(update))
    }
}

fn capture_image() -> io::Result<Option<CursorImage>> {
    let mut cursor = CURSORINFO {
        cbSize: u32::try_from(size_of::<CURSORINFO>()).map_err(io::Error::other)?,
        ..Default::default()
    };
    // UNTESTED: Query the interactive session's desktop, not a service desktop, to obtain its cursor.
    // Check capture in that session when changing desktop/thread setup.
    // SAFETY: The initialized output structure remains live for this call.
    unsafe { GetCursorInfo(&mut cursor) }.map_err(io::Error::other)?;
    if cursor.flags == CURSOR_SHOWING {
        CursorImage::from_handle(cursor.hCursor).map(Some)
    } else {
        Ok(None)
    }
}

#[cfg_attr(feature = "__test", visibility::make(pub))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct CursorImage {
    width: u16,
    height: u16,
    hot_x: u16,
    hot_y: u16,
    alpha: bool,
    xor: Vec<u8>,
    and: Vec<u8>,
}

impl CursorImage {
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    fn from_handle(cursor: HCURSOR) -> io::Result<Self> {
        let mut info = ICONINFO::default();
        // SAFETY: GetIconInfo accepts a shared cursor handle and creates owned bitmap copies.
        unsafe { GetIconInfo(HICON(cursor.0), &mut info) }.map_err(io::Error::other)?;
        let bitmaps = IconBitmaps(info);
        let mask_size = bitmap_size(bitmaps.0.hbmMask)?;
        let monochrome = bitmaps.0.hbmColor.is_invalid();
        let (width, height) = if monochrome {
            if mask_size.1 % 2 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid monochrome cursor mask height",
                ));
            }
            (mask_size.0, mask_size.1 / 2)
        } else {
            bitmap_size(bitmaps.0.hbmColor)?
        };
        let width = u16::try_from(width).map_err(io::Error::other)?;
        let height = u16::try_from(height).map_err(io::Error::other)?;
        let hot_x = u16::try_from(bitmaps.0.xHotspot).map_err(io::Error::other)?;
        let hot_y = u16::try_from(bitmaps.0.yHotspot).map_err(io::Error::other)?;
        validate_geometry(width, height, hot_x, hot_y)?;
        if mask_size != (i32::from(width), i32::from(height) * if monochrome { 2 } else { 1 }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "native cursor mask dimensions do not match its image",
            ));
        }

        // SAFETY: Creating a memory DC has no pointer preconditions.
        let dc = MemoryDc(unsafe { CreateCompatibleDC(None) });
        if dc.0.is_invalid() {
            return Err(io::Error::other("failed to create cursor bitmap DC"));
        }
        let mask = read_bitmap(dc.0, bitmaps.0.hbmMask, mask_size.0, mask_size.1, 1)?;
        let color = if monochrome {
            None
        } else {
            Some(read_bitmap(
                dc.0,
                bitmaps.0.hbmColor,
                i32::from(width),
                i32::from(height),
                32,
            )?)
        };
        Self::from_bitmaps(width, height, hot_x, hot_y, &mask, color.as_deref())
    }

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    fn from_bitmaps(
        width: u16,
        height: u16,
        hot_x: u16,
        hot_y: u16,
        mask: &[u8],
        color: Option<&[u8]>,
    ) -> io::Result<Self> {
        validate_geometry(width, height, hot_x, hot_y)?;
        let w = usize::from(width);
        let h = usize::from(height);
        let mask_stride = w.div_ceil(32) * 4;
        if mask.len() != mask_stride * h * if color.is_some() { 1 } else { 2 } {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid native cursor mask length",
            ));
        }
        if let Some(color) = color
            && color.len() != w * h * 4
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid native cursor color length",
            ));
        }
        let alpha = color.is_some_and(|data| data.chunks_exact(4).any(|pixel| pixel[3] != 0));
        let xor_stride = (w * if alpha { 4 } else { 3 }).next_multiple_of(2);
        let and_stride = w.div_ceil(16) * 2;
        let mut xor = vec![0; xor_stride * h];
        let mut and = if alpha { Vec::new() } else { vec![0; and_stride * h] };
        for row in 0..h {
            let dest = h - row - 1;
            if !alpha {
                and[dest * and_stride..(dest + 1) * and_stride]
                    .copy_from_slice(&mask[row * mask_stride..row * mask_stride + and_stride]);
            }
            for col in 0..w {
                if let Some(color) = color {
                    let pixel = &color[(row * w + col) * 4..][..4];
                    if alpha {
                        xor[dest * xor_stride + col * 4..][..4].copy_from_slice(pixel);
                    } else {
                        xor[dest * xor_stride + col * 3..][..3].copy_from_slice(&pixel[..3]);
                    }
                } else {
                    let bit = (mask[(row + h) * mask_stride + col / 8] >> (7 - col % 8)) & 1;
                    xor[dest * xor_stride + col * 3..][..3].fill(if bit == 0 { 0 } else { 255 });
                }
            }
        }
        Ok(Self {
            width,
            height,
            hot_x,
            hot_y,
            alpha,
            xor,
            and,
        })
    }

    fn update(&self) -> DisplayUpdate {
        if self.alpha {
            DisplayUpdate::RGBAPointer(RGBAPointer {
                cache_index: 0,
                width: self.width,
                height: self.height,
                hot_x: self.hot_x,
                hot_y: self.hot_y,
                data: self.xor.clone(),
            })
        } else {
            DisplayUpdate::ColorPointer(ColorPointer {
                cache_index: 0,
                width: self.width,
                height: self.height,
                hot_x: self.hot_x,
                hot_y: self.hot_y,
                and_mask: self.and.clone(),
                xor_mask: self.xor.clone(),
            })
        }
    }
}

fn validate_geometry(width: u16, height: u16, hot_x: u16, hot_y: u16) -> io::Result<()> {
    if !((1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native cursor exceeds the supported dimensions",
        ));
    }
    if !(hot_x < width && hot_y < height) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cursor hotspot is outside its image",
        ));
    }
    Ok(())
}

// UNTESTED: Repeated capture must release every copied bitmap and DC to avoid exhausting GDI handles.
// Check GDI object counts under repeated capture when changing handle ownership.
struct IconBitmaps(ICONINFO);

impl Drop for IconBitmaps {
    fn drop(&mut self) {
        for bitmap in [self.0.hbmMask, self.0.hbmColor] {
            if !bitmap.is_invalid() {
                // SAFETY: GetIconInfo created these bitmap handles for this owner.
                let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            }
        }
    }
}

struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns the DC created by CreateCompatibleDC, or an invalid handle.
        let _ = unsafe { DeleteDC(self.0) };
    }
}

fn bitmap_size(bitmap: HBITMAP) -> io::Result<(i32, i32)> {
    let mut object = BITMAP::default();
    // SAFETY: The output is a live BITMAP and the handle belongs to IconBitmaps.
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            i32::try_from(size_of::<BITMAP>()).map_err(io::Error::other)?,
            Some(core::ptr::from_mut(&mut object).cast()),
        )
    };
    if usize::try_from(copied).map_err(io::Error::other)? != size_of::<BITMAP>() {
        return Err(io::Error::other("failed to query native cursor bitmap"));
    }
    if !(0 < object.bmWidth && 0 < object.bmHeight) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid native cursor bitmap dimensions",
        ));
    }
    Ok((object.bmWidth, object.bmHeight))
}

fn read_bitmap(dc: HDC, bitmap: HBITMAP, width: i32, height: i32, bpp: u16) -> io::Result<Vec<u8>> {
    #[repr(C)]
    struct BitmapInfo {
        header: BITMAPINFOHEADER,
        colors: [RGBQUAD; 2],
    }
    let mut info = BitmapInfo {
        header: BITMAPINFOHEADER {
            biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).map_err(io::Error::other)?,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: bpp,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        colors: [
            RGBQUAD::default(),
            RGBQUAD {
                rgbBlue: 255,
                rgbGreen: 255,
                rgbRed: 255,
                rgbReserved: 0,
            },
        ],
    };
    let stride = (usize::try_from(width).map_err(io::Error::other)? * usize::from(bpp)).div_ceil(32) * 4;
    let mut bytes = vec![0; stride * usize::try_from(height).map_err(io::Error::other)?];
    // SAFETY: Dimensions are bounded before allocation, the bitmap is not selected into a DC,
    // and the repr(C) information buffer includes the monochrome palette.
    let rows = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            u32::try_from(height).map_err(io::Error::other)?,
            Some(bytes.as_mut_ptr().cast()),
            core::ptr::from_mut(&mut info).cast::<BITMAPINFO>(),
            DIB_RGB_COLORS,
        )
    };
    if rows != height {
        return Err(io::Error::other("failed to read native cursor bitmap"));
    }
    Ok(bytes)
}
