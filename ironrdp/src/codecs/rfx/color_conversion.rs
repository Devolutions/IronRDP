#[cfg(test)]
mod tests;

use std::cmp::{max, min};
use std::io::{self, Write};

const DIVISOR: f32 = (1 << 16) as f32;
const ALPHA: u8 = 255;

pub fn ycbcr_to_rgb(input: YCbCrBuffer<'_>, mut output: &mut [u8]) -> io::Result<()> {
    for ycbcr in input {
        let pixel = Rgb::from(ycbcr);

        output.write_all(&[pixel.b, pixel.g, pixel.r, ALPHA])?;
    }

    Ok(())
}

fn clip(v: i32) -> u8 {
    min(max(v, 0), 255) as u8
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
