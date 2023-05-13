use std::io;

use byteorder::WriteBytesExt;
use ironrdp_pdu::geometry::Rectangle;
use num_derive::ToPrimitive;
use num_traits::ToPrimitive as _;

const MIN_ALPHA: u8 = 0x00;
const MAX_ALPHA: u8 = 0xff;

pub struct ImageRegionMut<'a> {
    pub region: Rectangle,
    pub step: u16,
    pub pixel_format: PixelFormat,
    pub data: &'a mut [u8],
}

pub struct ImageRegion<'a> {
    pub region: Rectangle,
    pub step: u16,
    pub pixel_format: PixelFormat,
    pub data: &'a [u8],
}

impl<'a> ImageRegion<'a> {
    pub fn copy_to(&self, other: &mut ImageRegionMut<'_>) -> io::Result<()> {
        let width = usize::from(other.region.width());
        let height = usize::from(other.region.height());

        let dst_point = Point {
            x: usize::from(other.region.left),
            y: usize::from(other.region.top),
        };
        let src_point = Point {
            x: usize::from(self.region.left),
            y: usize::from(self.region.top),
        };

        let src_byte = usize::from(self.pixel_format.bytes_per_pixel());
        let dst_byte = usize::from(other.pixel_format.bytes_per_pixel());
        let dst_width = width * dst_byte;

        let src_step = if self.step == 0 {
            width * src_byte
        } else {
            usize::from(self.step)
        };
        let dst_step = if other.step == 0 {
            width * dst_byte
        } else {
            usize::from(other.step)
        };

        if self.pixel_format.eq_no_alpha(other.pixel_format) {
            for y in 0..height {
                let src_start = (y + src_point.y) * src_step + src_point.x * src_byte;
                let dst_start = (y + dst_point.y) * dst_step + dst_point.x * dst_byte;
                other.data[dst_start..dst_start + dst_width]
                    .clone_from_slice(&self.data[src_start..src_start + dst_width]);
            }
        } else {
            for y in 0..height {
                let src = &self.data[((y + src_point.y) * src_step)..];
                let dst = &mut other.data[((y + dst_point.y) * dst_step)..];

                for x in 0..width {
                    let color = self.pixel_format.read_color(&src[((x + src_point.x) * src_byte)..])?;
                    other
                        .pixel_format
                        .write_color(color, &mut dst[((x + dst_point.x) * dst_byte)..])?;
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ToPrimitive)]
pub enum PixelFormat {
    ARgb32 = 536_971_400,
    XRgb32 = 536_938_632,
    ABgr32 = 537_036_936,
    XBgr32 = 537_004_168,
    BgrA32 = 537_168_008,
    BgrX32 = 537_135_240,
    RgbA32 = 537_102_472,
    RgbX32 = 537_069_704,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> u8 {
        match self {
            Self::ARgb32
            | Self::XRgb32
            | Self::ABgr32
            | Self::XBgr32
            | Self::BgrA32
            | Self::BgrX32
            | Self::RgbA32
            | Self::RgbX32 => 4,
        }
    }

    pub fn eq_no_alpha(self, other: Self) -> bool {
        let mask = !(8 << 12);

        (self.to_u32().unwrap() & mask) == (other.to_u32().unwrap() & mask)
    }

    pub fn read_color(self, buffer: &[u8]) -> io::Result<Rgba> {
        match self {
            Self::ARgb32
            | Self::XRgb32
            | Self::ABgr32
            | Self::XBgr32
            | Self::BgrA32
            | Self::BgrX32
            | Self::RgbA32
            | Self::RgbX32 => {
                if buffer.len() < 4 {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "input buffer is not large enough (this is a bug)",
                    ))
                } else {
                    let color = &buffer[..4];

                    match self {
                        Self::ARgb32 => Ok(Rgba {
                            a: color[0],
                            r: color[1],
                            g: color[2],
                            b: color[3],
                        }),
                        Self::XRgb32 => Ok(Rgba {
                            a: MAX_ALPHA,
                            r: color[1],
                            g: color[2],
                            b: color[3],
                        }),
                        Self::ABgr32 => Ok(Rgba {
                            a: color[0],
                            b: color[1],
                            g: color[2],
                            r: color[3],
                        }),
                        Self::XBgr32 => Ok(Rgba {
                            a: MAX_ALPHA,
                            b: color[1],
                            g: color[2],
                            r: color[3],
                        }),
                        Self::BgrA32 => Ok(Rgba {
                            b: color[0],
                            g: color[1],
                            r: color[2],
                            a: color[3],
                        }),
                        Self::BgrX32 => Ok(Rgba {
                            b: color[0],
                            g: color[1],
                            r: color[2],
                            a: MAX_ALPHA,
                        }),
                        Self::RgbA32 => Ok(Rgba {
                            r: color[0],
                            g: color[1],
                            b: color[2],
                            a: color[3],
                        }),
                        Self::RgbX32 => Ok(Rgba {
                            r: color[0],
                            g: color[1],
                            b: color[2],
                            a: MAX_ALPHA,
                        }),
                    }
                }
            }
        }
    }

    pub fn write_color(self, color: Rgba, mut buffer: &mut [u8]) -> io::Result<()> {
        match self {
            Self::ARgb32 => {
                buffer.write_u8(color.a)?;
                buffer.write_u8(color.r)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.b)?;
            }
            Self::XRgb32 => {
                buffer.write_u8(MIN_ALPHA)?;
                buffer.write_u8(color.r)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.b)?;
            }
            Self::ABgr32 => {
                buffer.write_u8(color.a)?;
                buffer.write_u8(color.b)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.r)?;
            }
            Self::XBgr32 => {
                buffer.write_u8(MIN_ALPHA)?;
                buffer.write_u8(color.b)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.r)?;
            }
            Self::BgrA32 => {
                buffer.write_u8(color.b)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.r)?;
                buffer.write_u8(color.a)?;
            }
            Self::BgrX32 => {
                buffer.write_u8(color.b)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.r)?;
                buffer.write_u8(MIN_ALPHA)?;
            }
            Self::RgbA32 => {
                buffer.write_u8(color.r)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.b)?;
                buffer.write_u8(color.a)?;
            }
            Self::RgbX32 => {
                buffer.write_u8(color.r)?;
                buffer.write_u8(color.g)?;
                buffer.write_u8(color.b)?;
                buffer.write_u8(MIN_ALPHA)?;
            }
        }

        Ok(())
    }
}

struct Point {
    pub x: usize,
    pub y: usize,
}

pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}
