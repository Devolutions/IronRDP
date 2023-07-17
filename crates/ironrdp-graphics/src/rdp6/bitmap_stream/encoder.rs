use std::marker::PhantomData;

use ironrdp_pdu::{
    bitmap::rdp6::{BitmapStream as BitmapStreamPdu, ColorPlanes},
    encode,
};
use thiserror::Error;

use crate::rdp6::rle::{compress_8bpp_plane, RleError};

#[derive(Debug, Error)]
pub enum BitmapEncodeError {
    #[error("Failed to encode bitmap")]
    InvalidBitmap,
    #[error("Failed to rle compress")]
    RleFailed(RleError),
}

pub trait PixelFormat {
    const STRIDE: usize;

    fn r(pixel: &[u8]) -> u8;
    fn g(pixel: &[u8]) -> u8;
    fn b(pixel: &[u8]) -> u8;
}

pub trait PixelAlpha: PixelFormat {
    fn a(pixel: &[u8]) -> u8;
}

pub struct RGBFormat;
pub struct ARGBFormat;
pub struct RGBAFormat;

impl PixelFormat for RGBFormat {
    const STRIDE: usize = 3;

    fn r(pixel: &[u8]) -> u8 {
        pixel[0]
    }

    fn g(pixel: &[u8]) -> u8 {
        pixel[1]
    }

    fn b(pixel: &[u8]) -> u8 {
        pixel[2]
    }
}

impl PixelFormat for RGBAFormat {
    const STRIDE: usize = 4;

    fn r(pixel: &[u8]) -> u8 {
        pixel[0]
    }

    fn g(pixel: &[u8]) -> u8 {
        pixel[1]
    }

    fn b(pixel: &[u8]) -> u8 {
        pixel[2]
    }
}

impl PixelAlpha for RGBAFormat {
    fn a(pixel: &[u8]) -> u8 {
        pixel[3]
    }
}

impl PixelFormat for ARGBFormat {
    const STRIDE: usize = 4;

    fn r(pixel: &[u8]) -> u8 {
        pixel[1]
    }

    fn g(pixel: &[u8]) -> u8 {
        pixel[2]
    }

    fn b(pixel: &[u8]) -> u8 {
        pixel[3]
    }
}

impl PixelAlpha for ARGBFormat {
    fn a(pixel: &[u8]) -> u8 {
        pixel[0]
    }
}

impl BitmapEncodeError {
    fn invalid_bitmap<E>(_: E) -> Self {
        Self::InvalidBitmap
    }

    fn rle(e: RleError) -> Self {
        Self::RleFailed(e)
    }
}

pub struct BitmapStreamEncoder<F> {
    width: usize,
    height: usize,
    planes_buffer: Vec<u8>,
    _format: PhantomData<F>,
}

impl<F> BitmapStreamEncoder<F>
where
    F: PixelFormat,
{
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            planes_buffer: Vec::new(),
            _format: PhantomData,
        }
    }

    pub fn encode_bitmap(&mut self, src: &[u8], dst: &mut [u8], rle: bool) -> Result<usize, BitmapEncodeError> {
        self.planes_buffer.clear();

        let r = src.chunks(F::STRIDE).map(F::r);
        let g = src.chunks(F::STRIDE).map(F::g);
        let b = src.chunks(F::STRIDE).map(F::b);

        match rle {
            true => {
                let mut cursor = std::io::Cursor::new(&mut self.planes_buffer);
                compress_8bpp_plane(r, &mut cursor, self.width, self.height).map_err(BitmapEncodeError::rle)?;
                compress_8bpp_plane(g, &mut cursor, self.width, self.height).map_err(BitmapEncodeError::rle)?;
                compress_8bpp_plane(b, &mut cursor, self.width, self.height).map_err(BitmapEncodeError::rle)?;
            }

            false => {
                self.planes_buffer.extend(r.chain(g).chain(b));
            }
        }

        let header = BitmapStreamPdu {
            enable_rle_compression: rle,
            use_alpha: false,
            color_planes: ColorPlanes::Argb {
                data: &self.planes_buffer,
            },
        };

        encode::<BitmapStreamPdu>(&header, dst).map_err(BitmapEncodeError::invalid_bitmap)
    }
}

impl<F> BitmapStreamEncoder<F>
where
    F: PixelFormat + PixelAlpha,
{
    pub fn encode_bitmap_alpha(&mut self, src: &[u8], dst: &mut [u8], rle: bool) -> Result<usize, BitmapEncodeError> {
        self.planes_buffer.clear();

        let r = src.chunks(F::STRIDE).map(F::r);
        let g = src.chunks(F::STRIDE).map(F::g);
        let b = src.chunks(F::STRIDE).map(F::b);
        let a = src.chunks(F::STRIDE).map(F::a);

        match rle {
            true => {
                let mut cursor = std::io::Cursor::new(&mut self.planes_buffer);
                compress_8bpp_plane(a, &mut cursor, self.width, self.height).map_err(BitmapEncodeError::rle)?;
                compress_8bpp_plane(r, &mut cursor, self.width, self.height).map_err(BitmapEncodeError::rle)?;
                compress_8bpp_plane(g, &mut cursor, self.width, self.height).map_err(BitmapEncodeError::rle)?;
                compress_8bpp_plane(b, &mut cursor, self.width, self.height).map_err(BitmapEncodeError::rle)?;
            }

            false => {
                self.planes_buffer.extend(a.chain(r).chain(g).chain(b));
            }
        }

        let header = BitmapStreamPdu {
            enable_rle_compression: rle,
            use_alpha: true,
            color_planes: ColorPlanes::Argb {
                data: &self.planes_buffer,
            },
        };

        encode::<BitmapStreamPdu>(&header, dst).map_err(BitmapEncodeError::invalid_bitmap)
    }
}
