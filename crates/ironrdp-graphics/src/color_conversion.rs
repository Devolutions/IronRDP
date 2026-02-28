use std::io;

use yuv::{
    rdp_abgr_to_yuv444, rdp_argb_to_yuv444, rdp_bgra_to_yuv444, rdp_rgba_to_yuv444, rdp_yuv444_to_argb,
    rdp_yuv444_to_rgba, BufferStoreMut, YuvError, YuvPlanarImage, YuvPlanarImageMut,
};

use crate::image_processing::PixelFormat;

// FIXME: used for the test suite, we may want to drop it
pub fn ycbcr_to_argb(input: YCbCrBuffer<'_>, output: &mut [u8]) -> io::Result<()> {
    let len = u32::try_from(output.len()).map_err(io::Error::other)?;
    let width = len / 4;
    let planar = YuvPlanarImage {
        y_plane: input.y,
        y_stride: width,
        u_plane: input.cb,
        u_stride: width,
        v_plane: input.cr,
        v_stride: width,
        width,
        height: 1,
    };
    rdp_yuv444_to_argb(&planar, output, len).map_err(io::Error::other)
}

pub fn ycbcr_to_rgba(input: YCbCrBuffer<'_>, output: &mut [u8]) -> io::Result<()> {
    let len = u32::try_from(output.len()).map_err(io::Error::other)?;
    let width = len / 4;
    let planar = YuvPlanarImage {
        y_plane: input.y,
        y_stride: width,
        u_plane: input.cb,
        u_stride: width,
        v_plane: input.cr,
        v_stride: width,
        width,
        height: 1,
    };
    rdp_yuv444_to_rgba(&planar, output, len).map_err(io::Error::other)
}

/// # Panics
///
/// - Panics if `width` > 64.
/// - Panics if `height` > 64.
#[expect(clippy::too_many_arguments)]
pub fn to_64x64_ycbcr_tile(
    input: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    format: PixelFormat,
    y: &mut [i16; 64 * 64],
    cb: &mut [i16; 64 * 64],
    cr: &mut [i16; 64 * 64],
) -> Result<(), YuvError> {
    assert!(width <= 64);
    assert!(height <= 64);

    let y_plane = BufferStoreMut::Borrowed(y);
    let u_plane = BufferStoreMut::Borrowed(cb);
    let v_plane = BufferStoreMut::Borrowed(cr);
    let mut plane = YuvPlanarImageMut {
        y_plane,
        y_stride: 64,
        u_plane,
        u_stride: 64,
        v_plane,
        v_stride: 64,
        width,
        height,
    };

    match format {
        PixelFormat::RgbA32 | PixelFormat::RgbX32 => rdp_rgba_to_yuv444(&mut plane, input, stride),
        PixelFormat::ARgb32 | PixelFormat::XRgb32 => rdp_argb_to_yuv444(&mut plane, input, stride),
        PixelFormat::BgrA32 | PixelFormat::BgrX32 => rdp_bgra_to_yuv444(&mut plane, input, stride),
        PixelFormat::ABgr32 | PixelFormat::XBgr32 => rdp_abgr_to_yuv444(&mut plane, input, stride),
    }
}

/// Convert a 15-bit RDP color (RGB555) to RGB. Input value should be represented in
/// little-endian format.
///
/// Layout: `[0, R4:R0, G4:G0, B4:B0]` -- MSB unused, 5 bits per channel.
pub fn rdp_15bit_to_rgb(color: u16) -> [u8; 3] {
    #[expect(clippy::missing_panics_doc, reason = "unreachable panic (checked integer underflow)")]
    let out = {
        let r = u8::try_from(((((color >> 10) & 0x1f) * 527) + 23) >> 6).expect("max possible value is 255");
        let g = u8::try_from(((((color >> 5) & 0x1f) * 527) + 23) >> 6).expect("max possible value is 255");
        let b = u8::try_from((((color & 0x1f) * 527) + 23) >> 6).expect("max possible value is 255");
        [r, g, b]
    };

    out
}

/// Convert a 16-bit RDP color (RGB565) to RGB. Input value should be represented in
/// little-endian format.
pub fn rdp_16bit_to_rgb(color: u16) -> [u8; 3] {
    #[expect(clippy::missing_panics_doc, reason = "unreachable panic (checked integer underflow)")]
    let out = {
        let r = u8::try_from(((((color >> 11) & 0x1f) * 527) + 23) >> 6).expect("max possible value is 255");
        let g = u8::try_from(((((color >> 5) & 0x3f) * 259) + 33) >> 6).expect("max possible value is 255");
        let b = u8::try_from((((color & 0x1f) * 527) + 23) >> 6).expect("max possible value is 255");
        [r, g, b]
    };

    out
}

#[derive(Debug)]
pub struct YCbCrBuffer<'a> {
    pub y: &'a [i16],
    pub cb: &'a [i16],
    pub cr: &'a [i16],
}

impl Iterator for YCbCrBuffer<'_> {
    type Item = YCbCr;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.y.is_empty() && !self.cb.is_empty() && !self.cr.is_empty() {
            let y = self.y[0];
            let cb = self.cb[0];
            let cr = self.cr[0];

            self.y = &self.y[1..];
            self.cb = &self.cb[1..];
            self.cr = &self.cr[1..];

            Some(YCbCr { y, cb, cr })
        } else {
            None
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct YCbCr {
    pub y: i16,
    pub cb: i16,
    pub cr: i16,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdp_15bit_black() {
        assert_eq!(rdp_15bit_to_rgb(0x0000), [0, 0, 0]);
    }

    #[test]
    fn rdp_15bit_white() {
        assert_eq!(rdp_15bit_to_rgb(0x7FFF), [255, 255, 255]);
    }

    #[test]
    fn rdp_15bit_pure_red() {
        // R=31, G=0, B=0: 0_11111_00000_00000 = 0x7C00
        assert_eq!(rdp_15bit_to_rgb(0x7C00), [255, 0, 0]);
    }

    #[test]
    fn rdp_15bit_pure_green() {
        // R=0, G=31, B=0: 0_00000_11111_00000 = 0x03E0
        assert_eq!(rdp_15bit_to_rgb(0x03E0), [0, 255, 0]);
    }

    #[test]
    fn rdp_15bit_pure_blue() {
        // R=0, G=0, B=31: 0_00000_00000_11111 = 0x001F
        assert_eq!(rdp_15bit_to_rgb(0x001F), [0, 0, 255]);
    }

    #[test]
    fn rdp_16bit_black() {
        assert_eq!(rdp_16bit_to_rgb(0x0000), [0, 0, 0]);
    }

    #[test]
    fn rdp_16bit_white() {
        assert_eq!(rdp_16bit_to_rgb(0xFFFF), [255, 255, 255]);
    }

    #[test]
    fn rdp_16bit_pure_red() {
        // R=31, G=0, B=0: 11111_000000_00000 = 0xF800
        assert_eq!(rdp_16bit_to_rgb(0xF800), [255, 0, 0]);
    }

    #[test]
    fn rdp_16bit_pure_green() {
        // R=0, G=63, B=0: 00000_111111_00000 = 0x07E0
        assert_eq!(rdp_16bit_to_rgb(0x07E0), [0, 255, 0]);
    }

    #[test]
    fn rdp_16bit_pure_blue() {
        // R=0, G=0, B=31: 00000_000000_11111 = 0x001F
        assert_eq!(rdp_16bit_to_rgb(0x001F), [0, 0, 255]);
    }
}
