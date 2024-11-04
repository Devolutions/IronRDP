use std::cmp::min;
use std::io::{self, Write};

use crate::image_processing::PixelFormat;

const ALPHA: u8 = 255;

pub fn ycbcr_to_bgra(input: YCbCrBuffer<'_>, mut output: &mut [u8]) -> io::Result<()> {
    for ycbcr in input {
        let pixel = Rgb::from(ycbcr);

        output.write_all(&[pixel.b, pixel.g, pixel.r, ALPHA])?;
    }

    Ok(())
}

fn iter_to_ycbcr<'a, I, C>(input: I, y: &mut [i16], cb: &mut [i16], cr: &mut [i16], conv: C)
where
    I: ExactSizeIterator<Item = &'a [u8]>,
    C: Fn(&[u8]) -> Rgb,
{
    for (i, pixel) in input.into_iter().enumerate() {
        let pixel = YCbCr::from(conv(pixel));

        y[i] = pixel.y;
        cb[i] = pixel.cb;
        cr[i] = pixel.cr;
    }
}

fn xrgb_to_rgb(pixel: &[u8]) -> Rgb {
    Rgb {
        r: pixel[1],
        g: pixel[2],
        b: pixel[3],
    }
}

fn xbgr_to_rgb(pixel: &[u8]) -> Rgb {
    Rgb {
        b: pixel[1],
        g: pixel[2],
        r: pixel[3],
    }
}

fn bgrx_to_rgb(pixel: &[u8]) -> Rgb {
    Rgb {
        b: pixel[0],
        g: pixel[1],
        r: pixel[2],
    }
}

fn rgbx_to_rgb(pixel: &[u8]) -> Rgb {
    Rgb {
        r: pixel[0],
        g: pixel[1],
        b: pixel[2],
    }
}

const fn pixel_format_to_rgb_fn(format: PixelFormat) -> fn(&[u8]) -> Rgb {
    match format {
        PixelFormat::ARgb32 | PixelFormat::XRgb32 => xrgb_to_rgb,
        PixelFormat::ABgr32 | PixelFormat::XBgr32 => xbgr_to_rgb,
        PixelFormat::BgrA32 | PixelFormat::BgrX32 => bgrx_to_rgb,
        PixelFormat::RgbA32 | PixelFormat::RgbX32 => rgbx_to_rgb,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn to_ycbcr(
    mut input: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    mut y: &mut [i16],
    mut cb: &mut [i16],
    mut cr: &mut [i16],
) {
    let to_rgb = pixel_format_to_rgb_fn(format);
    let bpp = format.bytes_per_pixel() as usize;

    for _ in 0..height {
        iter_to_ycbcr(input[..width * bpp].chunks_exact(bpp), y, cb, cr, to_rgb);
        input = &input[stride..];
        y = &mut y[width..];
        cb = &mut cb[width..];
        cr = &mut cr[width..];
    }
}

struct TileIterator<'a> {
    slice: &'a [u8],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    stride: usize,
    bpp: usize,
}

impl<'a> TileIterator<'a> {
    fn new(slice: &'a [u8], width: usize, height: usize, stride: usize, bpp: usize) -> Self {
        assert!(width >= 1);
        assert!(height >= 1);

        Self {
            slice,
            x: 0,
            y: 0,
            width,
            height,
            stride,
            bpp,
        }
    }
}

impl<'a> Iterator for TileIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        // create 64x64 tiles
        if self.y >= 64 {
            return None;
        }

        // repeat the last column & line if necessary
        let y = min(self.y, self.height - 1);
        let x = min(self.x, self.width - 1);
        let pos = y * self.stride + x * self.bpp;

        self.x += 1;
        if self.x >= 64 {
            self.x = 0;
            self.y += 1;
        }

        Some(&self.slice[pos..pos + self.bpp])
    }
}

impl ExactSizeIterator for TileIterator<'_> {
    fn len(&self) -> usize {
        64 * 64
    }
}

#[allow(clippy::too_many_arguments)]
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

    let to_rgb = pixel_format_to_rgb_fn(format);
    let bpp = format.bytes_per_pixel() as usize;

    let input = TileIterator::new(input, width, height, stride, bpp);
    iter_to_ycbcr(input, y, cb, cr, to_rgb);
}

/// Convert a 16-bit RDP color to RGB representation. Input value should be represented in
/// little-endian format.
pub fn rdp_16bit_to_rgb(color: u16) -> [u8; 3] {
    let r = (((((color >> 11) & 0x1f) * 527) + 23) >> 6) as u8;
    let g = (((((color >> 5) & 0x3f) * 259) + 33) >> 6) as u8;
    let b = ((((color & 0x1f) * 527) + 23) >> 6) as u8;
    [r, g, b]
}

fn clip(v: i32) -> u8 {
    v.clamp(0, 255) as u8
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

impl From<YCbCr> for Rgb {
    fn from(YCbCr { y, cb, cr }: YCbCr) -> Self {
        // We scale the factors by << 16 into 32-bit integers in order to
        // avoid slower floating point multiplications.  Since the final
        // result needs to be scaled by >> 5 we will extract only the
        // upper 11 bits (>> 21) from the final sum.
        // Hence we also have to scale the other terms of the sum by << 16.
        const DIVISOR: f32 = (1 << 16) as f32;

        let y = i32::from(y);
        let cb = i32::from(cb);
        let cr = i32::from(cr);

        let yy = (y + 4096) << 16;
        let cr_r = cr.overflowing_mul((1.402_525 * DIVISOR) as i32).0;
        let cb_g = cb.overflowing_mul((0.343_730 * DIVISOR) as i32).0;
        let cr_g = cr.overflowing_mul((0.714_401 * DIVISOR) as i32).0;
        let cb_b = cb.overflowing_mul((1.769_905 * DIVISOR) as i32).0;
        let cr_b = cb.overflowing_mul((0.000_013 * DIVISOR) as i32).0;

        let r = clip((yy.overflowing_add(cr_r).0) >> 21);
        let g = clip((yy.overflowing_sub(cb_g).0.overflowing_sub(cr_g).0) >> 21);
        let b = clip((yy.overflowing_add(cb_b).0.overflowing_add(cr_b).0) >> 21);

        Self { r, g, b }
    }
}

impl From<Rgb> for YCbCr {
    fn from(Rgb { r, g, b }: Rgb) -> Self {
        // We scale the factors by << 15 into 32-bit integers in order
        // to avoid slower floating point multiplications.  Since the
        // terms need to be scaled by << 5 we simply scale the final
        // sum by >> 10
        const DIVISOR: f32 = (1 << 15) as f32;
        const Y_R: i32 = (0.299 * DIVISOR) as i32;
        const Y_G: i32 = (0.587 * DIVISOR) as i32;
        const Y_B: i32 = (0.114 * DIVISOR) as i32;
        const CB_R: i32 = (0.168_935 * DIVISOR) as i32;
        const CB_G: i32 = (0.331_665 * DIVISOR) as i32;
        const CB_B: i32 = (0.500_59 * DIVISOR) as i32;
        const CR_R: i32 = (0.499_813 * DIVISOR) as i32;
        const CR_G: i32 = (0.418_531 * DIVISOR) as i32;
        const CR_B: i32 = (0.081_282 * DIVISOR) as i32;

        let r = i32::from(r);
        let g = i32::from(g);
        let b = i32::from(b);

        let y_r = r.overflowing_mul(Y_R).0;
        let y_g = g.overflowing_mul(Y_G).0;
        let y_b = b.overflowing_mul(Y_B).0;
        let y = y_r.overflowing_add(y_g).0.overflowing_add(y_b).0 >> 10;

        let cb_r = r.overflowing_mul(CB_R).0;
        let cb_g = g.overflowing_mul(CB_G).0;
        let cb_b = b.overflowing_mul(CB_B).0;
        let cb = cb_b.overflowing_sub(cb_g).0.overflowing_sub(cb_r).0 >> 10;

        let cr_r = r.overflowing_mul(CR_R).0;
        let cr_g = g.overflowing_mul(CR_G).0;
        let cr_b = b.overflowing_mul(CR_B).0;
        let cr = cr_r.overflowing_sub(cr_g).0.overflowing_sub(cr_b).0 >> 10;

        Self {
            y: (y - 4096).clamp(-4096, 4095) as i16,
            cb: cb.clamp(-4096, 4095) as i16,
            cr: cr.clamp(-4096, 4095) as i16,
        }
    }
}
