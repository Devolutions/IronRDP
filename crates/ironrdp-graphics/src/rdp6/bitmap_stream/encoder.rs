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

pub trait ColorChannels {
    const STRIDE: usize;
    const R: usize;
    const G: usize;
    const B: usize;
}

pub trait AlphaChannel {
    const A: usize;
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

impl<T> PixelFormat for T
where
    T: ColorChannels,
{
    const STRIDE: usize = T::STRIDE;

    fn r(pixel: &[u8]) -> u8 {
        pixel[T::R]
    }

    fn g(pixel: &[u8]) -> u8 {
        pixel[T::G]
    }

    fn b(pixel: &[u8]) -> u8 {
        pixel[T::B]
    }
}

impl<T> PixelAlpha for T
where
    T: ColorChannels + AlphaChannel,
{
    fn a(pixel: &[u8]) -> u8 {
        pixel[T::A]
    }
}

pub struct RgbChannels;

impl ColorChannels for RgbChannels {
    const STRIDE: usize = 3;
    const R: usize = 0;
    const G: usize = 1;
    const B: usize = 2;
}

pub struct ARgbChannels;

impl ColorChannels for ARgbChannels {
    const STRIDE: usize = 4;
    const R: usize = 1;
    const G: usize = 2;
    const B: usize = 3;
}

impl AlphaChannel for ARgbChannels {
    const A: usize = 0;
}

pub struct RgbAChannels;

impl ColorChannels for RgbAChannels {
    const STRIDE: usize = 4;
    const R: usize = 0;
    const G: usize = 1;
    const B: usize = 2;
}

impl AlphaChannel for RgbAChannels {
    const A: usize = 3;
}

pub struct ABgrChannels;

impl ColorChannels for ABgrChannels {
    const STRIDE: usize = 4;
    const R: usize = 3;
    const G: usize = 2;
    const B: usize = 1;
}

impl AlphaChannel for ABgrChannels {
    const A: usize = 0;
}

pub struct BgrAChannels;

impl ColorChannels for BgrAChannels {
    const STRIDE: usize = 4;
    const R: usize = 2;
    const G: usize = 1;
    const B: usize = 0;
}

impl AlphaChannel for BgrAChannels {
    const A: usize = 3;
}

impl BitmapEncodeError {
    fn invalid_bitmap<E>(_: E) -> Self {
        Self::InvalidBitmap
    }

    fn rle(e: RleError) -> Self {
        Self::RleFailed(e)
    }
}

pub struct BitmapStreamEncoder {
    width: usize,
    height: usize,
    planes_buffer: Vec<u8>,
}

impl BitmapStreamEncoder {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            planes_buffer: Vec::new(),
        }
    }

    pub fn encode_channels_stream<R, G, B>(
        &mut self,
        (r, g, b): (R, G, B),
        dst: &mut [u8],
        rle: bool,
    ) -> Result<usize, BitmapEncodeError>
    where
        R: Iterator<Item = u8>,
        G: Iterator<Item = u8>,
        B: Iterator<Item = u8>,
    {
        self.planes_buffer.clear();

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

    pub fn encode_pixels_stream<'a, I, F>(
        &mut self,
        data: I,
        dst: &mut [u8],
        rle: bool,
    ) -> Result<usize, BitmapEncodeError>
    where
        F: PixelFormat,
        I: Iterator<Item = &'a [u8]> + Clone,
    {
        let r = data.clone().map(F::r);
        let g = data.clone().map(F::g);
        let b = data.map(F::b);

        self.encode_channels_stream((r, g, b), dst, rle)
    }


    pub fn encode_bitmap<F>(&mut self, src: &[u8], dst: &mut [u8], rle: bool) -> Result<usize, BitmapEncodeError>
    where
        F: PixelFormat,
    {
        let r = src.chunks(F::STRIDE).map(F::r);
        let g = src.chunks(F::STRIDE).map(F::g);
        let b = src.chunks(F::STRIDE).map(F::b);

        self.encode_channels_stream((r, g, b), dst, rle)
    }
}

impl BitmapStreamEncoder {
    pub fn encode_channels_stream_alpha<R, G, B, A>(
        &mut self,
        (r, g, b, a): (R, G, B, A),
        dst: &mut [u8],
        rle: bool,
    ) -> Result<usize, BitmapEncodeError>
    where
        R: Iterator<Item = u8>,
        G: Iterator<Item = u8>,
        B: Iterator<Item = u8>,
        A: Iterator<Item = u8>,
    {
        self.planes_buffer.clear();

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

    pub fn encode_bitmap_alpha<F>(&mut self, src: &[u8], dst: &mut [u8], rle: bool) -> Result<usize, BitmapEncodeError>
    where
        F: PixelFormat + PixelAlpha,
    {
        let r = src.chunks(F::STRIDE).map(F::r);
        let g = src.chunks(F::STRIDE).map(F::g);
        let b = src.chunks(F::STRIDE).map(F::b);
        let a = src.chunks(F::STRIDE).map(F::a);

        self.encode_channels_stream_alpha((r, g, b, a), dst, rle)
    }
}
