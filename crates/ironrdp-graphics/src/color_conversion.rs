use std::io;

use yuv::{
    rdp_abgr_to_yuv444, rdp_argb_to_yuv444, rdp_bgra_to_yuv444, rdp_rgba_to_yuv444, rdp_yuv444_to_argb,
    rdp_yuv444_to_rgba, BufferStoreMut, YuvPlanarImage, YuvPlanarImageMut,
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

#[expect(clippy::too_many_arguments)]
pub fn to_64x64_ycbcr_tile(
    input: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    y: &mut [i16; 64 * 64],
    cb: &mut [i16; 64 * 64],
    cr: &mut [i16; 64 * 64],
) {
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
        width: width.try_into().unwrap(),
        height: height.try_into().unwrap(),
    };

    let res = match format {
        PixelFormat::RgbA32 | PixelFormat::RgbX32 => rdp_rgba_to_yuv444(&mut plane, input, stride.try_into().unwrap()),
        PixelFormat::ARgb32 | PixelFormat::XRgb32 => rdp_argb_to_yuv444(&mut plane, input, stride.try_into().unwrap()),
        PixelFormat::BgrA32 | PixelFormat::BgrX32 => rdp_bgra_to_yuv444(&mut plane, input, stride.try_into().unwrap()),
        PixelFormat::ABgr32 | PixelFormat::XBgr32 => rdp_abgr_to_yuv444(&mut plane, input, stride.try_into().unwrap()),
    };
    res.unwrap();
}

/// Convert a 16-bit RDP color to RGB representation. Input value should be represented in
/// little-endian format.
pub fn rdp_16bit_to_rgb(color: u16) -> [u8; 3] {
    let r = (((((color >> 11) & 0x1f) * 527) + 23) >> 6) as u8;
    let g = (((((color >> 5) & 0x3f) * 259) + 33) >> 6) as u8;
    let b = ((((color & 0x1f) * 527) + 23) >> 6) as u8;
    [r, g, b]
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
