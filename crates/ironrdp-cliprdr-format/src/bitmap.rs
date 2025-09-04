use std::io::Cursor;

use ironrdp_core::{
    cast_int, ensure_fixed_part_size, invalid_field_err, Decode, DecodeResult, Encode, EncodeResult, ReadCursor,
    WriteCursor,
};

/// Maximum size of PNG image that could be placed on the clipboard.
const MAX_BUFFER_SIZE: usize = 64 * 1024 * 1024; // 64 MB

#[derive(Debug)]
pub enum BitmapError {
    Decode(ironrdp_core::DecodeError),
    Encode(ironrdp_core::EncodeError),
    Unsupported(&'static str),
    InvalidSize,
    BufferTooBig,
    WidthTooBig,
    HeightTooBig,
    PngEncode(png::EncodingError),
    PngDecode(png::DecodingError),
}

impl core::fmt::Display for BitmapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BitmapError::Decode(_error) => write!(f, "decoding error"),
            BitmapError::Encode(_error) => write!(f, "encoding error"),
            BitmapError::Unsupported(s) => write!(f, "unsupported bitmap: {s}"),
            BitmapError::InvalidSize => write!(f, "one of bitmap's dimensions is invalid"),
            BitmapError::BufferTooBig => write!(f, "buffer size required for allocation is too big"),
            BitmapError::WidthTooBig => write!(f, "image width is too big"),
            BitmapError::HeightTooBig => write!(f, "image height is too big"),
            BitmapError::PngEncode(_error) => write!(f, "PNG encoding error"),
            BitmapError::PngDecode(_error) => write!(f, "PNG decoding error"),
        }
    }
}

impl core::error::Error for BitmapError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            BitmapError::Decode(error) => Some(error),
            BitmapError::Encode(error) => Some(error),
            BitmapError::Unsupported(_) => None,
            BitmapError::InvalidSize => None,
            BitmapError::BufferTooBig => None,
            BitmapError::WidthTooBig => None,
            BitmapError::HeightTooBig => None,
            BitmapError::PngEncode(encoding_error) => Some(encoding_error),
            BitmapError::PngDecode(decoding_error) => Some(decoding_error),
        }
    }
}

impl From<png::EncodingError> for BitmapError {
    fn from(error: png::EncodingError) -> Self {
        BitmapError::PngEncode(error)
    }
}

impl From<png::DecodingError> for BitmapError {
    fn from(error: png::DecodingError) -> Self {
        BitmapError::PngDecode(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BitmapCompression(u32);

#[expect(dead_code)]
impl BitmapCompression {
    const RGB: Self = Self(0x0000);
    const RLE8: Self = Self(0x0001);
    const RLE4: Self = Self(0x0002);
    const BITFIELDS: Self = Self(0x0003);
    const JPEG: Self = Self(0x0004);
    const PNG: Self = Self(0x0005);
    const CMYK: Self = Self(0x000B);
    const CMYKRLE8: Self = Self(0x000C);
    const CMYKRLE4: Self = Self(0x000D);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ColorSpace(u32);

#[expect(dead_code)]
impl ColorSpace {
    const CALIBRATED_RGB: Self = Self(0x00000000);
    const SRGB: Self = Self(0x73524742);
    const WINDOWS: Self = Self(0x57696E20);
    const PROFILE_LINKED: Self = Self(0x4C494E4B);
    const PROFILE_EMBEDDED: Self = Self(0x4D424544);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BitmapIntent(u32);

#[expect(dead_code)]
impl BitmapIntent {
    const LCS_GM_ABS_COLORIMETRIC: Self = Self(0x00000008);
    const LCS_GM_BUSINESS: Self = Self(0x00000001);
    const LCS_GM_GRAPHICS: Self = Self(0x00000002);
    const LCS_GM_IMAGES: Self = Self(0x00000004);
}

type Fxpt2Dot30 = u32; // (LONG)

#[derive(Default)]
struct Ciexyz {
    x: Fxpt2Dot30,
    y: Fxpt2Dot30,
    z: Fxpt2Dot30,
}

#[derive(Default)]
struct CiexyzTriple {
    red: Ciexyz,
    green: Ciexyz,
    blue: Ciexyz,
}

impl CiexyzTriple {
    const NAME: &'static str = "CIEXYZTRIPLE";
    const FIXED_PART_SIZE: usize = 4 * 3 * 3; // 4(LONG) * 3(xyz) * 3(red, green, blue)
}

impl<'a> Decode<'a> for CiexyzTriple {
    fn decode(src: &mut ReadCursor<'a>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let red = Ciexyz {
            x: src.read_u32(),
            y: src.read_u32(),
            z: src.read_u32(),
        };

        let green = Ciexyz {
            x: src.read_u32(),
            y: src.read_u32(),
            z: src.read_u32(),
        };

        let blue = Ciexyz {
            x: src.read_u32(),
            y: src.read_u32(),
            z: src.read_u32(),
        };

        Ok(Self { red, green, blue })
    }
}

impl Encode for CiexyzTriple {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.red.x);
        dst.write_u32(self.red.y);
        dst.write_u32(self.red.z);

        dst.write_u32(self.green.x);
        dst.write_u32(self.green.y);
        dst.write_u32(self.green.z);

        dst.write_u32(self.blue.x);
        dst.write_u32(self.blue.y);
        dst.write_u32(self.blue.z);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// Header used in `CF_DIB` formats, part of [BITMAPINFO]
///
/// We don't use the optional `bmiColors` field, because it is only relevant for bitmaps with
/// bpp < 24, which are not supported yet, therefore only fixed part of the header is implemented.
///
/// [BITMAPINFO]: https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapinfo
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BitmapInfoHeader {
    /// INVARIANT: `width.abs() <= 10_000`
    width: i32,
    /// INVARIANT: `height.abs() <= 10_000`
    height: i32,
    /// INVARIANT: `bit_count <= 32`
    bit_count: u16,
    compression: BitmapCompression,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

impl BitmapInfoHeader {
    const FIXED_PART_SIZE: usize = 4 // biSize (DWORD)
        + 4 // biWidth (LONG)
        + 4 // biHeight (LONG)
        + 2 // biPlanes (WORD)
        + 2 // biBitCount (WORD)
        + 4 // biCompression (DWORD)
        + 4 // biSizeImage (DWORD)
        + 4 // biXPelsPerMeter (LONG)
        + 4 // biYPelsPerMeter (LONG)
        + 4 // biClrUsed (DWORD)
        + 4; // biClrImportant (DWORD)

    const NAME: &'static str = "BITMAPINFOHEADER";

    fn encode_with_size(&self, dst: &mut WriteCursor<'_>, size: u32) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(size);
        dst.write_i32(self.width);
        dst.write_i32(self.height);
        dst.write_u16(1); // biPlanes
        dst.write_u16(self.bit_count);
        dst.write_u32(self.compression.0);
        dst.write_u32(self.size_image);
        dst.write_i32(self.x_pels_per_meter);
        dst.write_i32(self.y_pels_per_meter);
        dst.write_u32(self.clr_used);
        dst.write_u32(self.clr_important);

        Ok(())
    }

    fn decode_with_size(src: &mut ReadCursor<'_>) -> DecodeResult<(Self, u32)> {
        ensure_fixed_part_size!(in: src);

        let size = src.read_u32();

        // NOTE: .abs() could panic on i32::MIN, therefore we have a check for it first.

        let width = src.read_i32();
        check_invariant(width != i32::MIN && width.abs() <= 10_000)
            .ok_or_else(|| invalid_field_err!("biWidth", "width is too big"))?;

        let height = src.read_i32();
        check_invariant(height != i32::MIN && height.abs() <= 10_000)
            .ok_or_else(|| invalid_field_err!("biHeight", "height is too big"))?;

        let planes = src.read_u16();
        if planes != 1 {
            return Err(invalid_field_err!("biPlanes", "invalid planes count"));
        }

        let bit_count = src.read_u16();
        check_invariant(bit_count <= 32).ok_or_else(|| invalid_field_err!("biBitCount", "invalid bit count"))?;

        let compression = BitmapCompression(src.read_u32());
        let size_image = src.read_u32();
        let x_pels_per_meter = src.read_i32();
        let y_pels_per_meter = src.read_i32();
        let clr_used = src.read_u32();
        let clr_important = src.read_u32();

        let header = Self {
            width,
            height,
            bit_count,
            compression,
            size_image,
            x_pels_per_meter,
            y_pels_per_meter,
            clr_used,
            clr_important,
        };

        Ok((header, size))
    }

    // INVARIANT: output (width) <= 10_000
    fn width(&self) -> u16 {
        let abs = self.width.abs();
        debug_assert!(abs <= 10_000);
        u16::try_from(abs).expect("Per the invariant on self.width, this cast is infallible")
    }

    // INVARIANT: output (height) <= 10_000
    fn height(&self) -> u16 {
        let abs = self.height.abs();
        debug_assert!(abs <= 10_000);
        u16::try_from(abs).expect("Per the invariant on self.height, this cast is infallible")
    }

    fn is_bottom_up(&self) -> bool {
        // When self.height is positive, the bitmap is defined as bottom-up.
        self.height >= 0
    }
}

impl Encode for BitmapInfoHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = cast_int!("biSize", Self::FIXED_PART_SIZE)?;
        self.encode_with_size(dst, size)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> Decode<'a> for BitmapInfoHeader {
    fn decode(src: &mut ReadCursor<'a>) -> DecodeResult<Self> {
        let (header, size) = Self::decode_with_size(src)?;
        let size: usize = cast_int!("biSize", size)?;

        if size != Self::FIXED_PART_SIZE {
            return Err(invalid_field_err!("biSize", "invalid V1 bitmap info header size"));
        }

        Ok(header)
    }
}

/// Header used in `CF_DIBV5` formats, defined as [BITMAPV5HEADER]
///
/// [BITMAPV5HEADER]: https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapv5header
struct BitmapV5Header {
    v1: BitmapInfoHeader,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
    color_space: ColorSpace,
    endpoints: CiexyzTriple,
    gamma_red: u32,
    gamma_green: u32,
    gamma_blue: u32,
    intent: BitmapIntent,
    profile_data: u32,
    profile_size: u32,
}

impl BitmapV5Header {
    const FIXED_PART_SIZE: usize = BitmapInfoHeader::FIXED_PART_SIZE // BITMAPV5HEADER
        + 4 // bV5RedMask (DWORD)
        + 4 // bV5GreenMask (DWORD)
        + 4 // bV5BlueMask (DWORD)
        + 4 // bV5AlphaMask (DWORD)
        + 4 // bV5CSType (DWORD)
        + CiexyzTriple::FIXED_PART_SIZE // bV5Endpoints (CIEXYZTRIPLE)
        + 4 // bV5GammaRed (DWORD)
        + 4 // bV5GammaGreen (DWORD)
        + 4 // bV5GammaBlue (DWORD)
        + 4 // bV5Intent (DWORD)
        + 4 // bV5ProfileData (DWORD)
        + 4 // bV5ProfileSize (DWORD)
        + 4; // bV5Reserved (DWORD)

    const NAME: &'static str = "BITMAPV5HEADER";
}

impl Encode for BitmapV5Header {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        let size = cast_int!("biSize", Self::FIXED_PART_SIZE)?;
        self.v1.encode_with_size(dst, size)?;

        dst.write_u32(self.red_mask);
        dst.write_u32(self.green_mask);
        dst.write_u32(self.blue_mask);
        dst.write_u32(self.alpha_mask);
        dst.write_u32(self.color_space.0);
        self.endpoints.encode(dst)?;
        dst.write_u32(self.gamma_red);
        dst.write_u32(self.gamma_green);
        dst.write_u32(self.gamma_blue);
        dst.write_u32(self.intent.0);
        dst.write_u32(self.profile_data);
        dst.write_u32(self.profile_size);
        dst.write_u32(0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> Decode<'a> for BitmapV5Header {
    fn decode(src: &mut ReadCursor<'a>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let (header_v1, size) = BitmapInfoHeader::decode_with_size(src)?;
        let size: usize = cast_int!("biSize", size)?;

        if size != Self::FIXED_PART_SIZE {
            return Err(invalid_field_err!("biSize", "invalid V5 bitmap info header size"));
        }

        let red_mask = src.read_u32();
        let green_mask = src.read_u32();
        let blue_mask = src.read_u32();
        let alpha_mask = src.read_u32();
        let color_space_type = ColorSpace(src.read_u32());
        let endpoints = CiexyzTriple::decode(src)?;
        let gamma_red = src.read_u32();
        let gamma_green = src.read_u32();
        let gamma_blue = src.read_u32();
        let intent = BitmapIntent(src.read_u32());
        let profile_data = src.read_u32();
        let profile_size = src.read_u32();
        let _reserved = src.read_u32();

        Ok(Self {
            v1: header_v1,
            red_mask,
            green_mask,
            blue_mask,
            alpha_mask,
            color_space: color_space_type,
            endpoints,
            gamma_red,
            gamma_green,
            gamma_blue,
            intent,
            profile_data,
            profile_size,
        })
    }
}

fn validate_v1_header(header: &BitmapInfoHeader) -> Result<(), BitmapError> {
    if header.width < 0 {
        return Err(BitmapError::Unsupported("negative width"));
    }

    if header.width == 0 || header.height == 0 {
        return Err(BitmapError::InvalidSize);
    }

    // In the modern world bitmaps with bpp < 24 are rare, and it is even more rare for the bitmaps
    // which are placed on the clipboard as DIBs, therefore we could safely skip the support for
    // such bitmaps.
    const SUPPORTED_BIT_COUNT: &[u16] = &[24, 32];

    if !SUPPORTED_BIT_COUNT.contains(&header.bit_count) {
        return Err(BitmapError::Unsupported("unsupported bit count"));
    }

    // This is only relevant for bitmaps with bpp < 24, which are not supported.
    if header.clr_used != 0 {
        return Err(BitmapError::Unsupported("color table is not supported"));
    }

    Ok(())
}

fn validate_v5_header(header: &BitmapV5Header) -> Result<(), BitmapError> {
    validate_v1_header(&header.v1)?;

    // We support only uncompressed DIB bitmaps as it is the most common case for clipboard-copied bitmaps.
    const DIBV5_SUPPORTED_COMPRESSION: &[BitmapCompression] = &[BitmapCompression::RGB, BitmapCompression::BITFIELDS];

    if !DIBV5_SUPPORTED_COMPRESSION.contains(&header.v1.compression) {
        return Err(BitmapError::Unsupported("unsupported compression"));
    }

    if header.v1.compression == BitmapCompression::BITFIELDS {
        // Currently, we only support the standard order, BGRA, for the bitfields compression.
        let is_bgr = header.red_mask == 0x00FF0000 && header.green_mask == 0x0000FF00 && header.blue_mask == 0x000000FF;

        // Note: when there is no alpha channel, the mask is 0x00000000 and we support this too.
        let is_supported_alpha = header.alpha_mask == 0 || header.alpha_mask == 0xFF000000;

        if !is_bgr || !is_supported_alpha {
            return Err(BitmapError::Unsupported(
                "non-standard color masks for `BITFIELDS` compression are not supported",
            ));
        }
    }

    const SUPPORTED_COLOR_SPACE: &[ColorSpace] = &[
        ColorSpace::SRGB,
        // Assume that Windows color space is sRGB, either way we don't have enough information on
        // the clipboard to convert it to other color spaces.
        ColorSpace::WINDOWS,
    ];

    if !SUPPORTED_COLOR_SPACE.contains(&header.color_space) {
        return Err(BitmapError::Unsupported("not supported color space"));
    }

    Ok(())
}

struct PngEncoderContext {
    bitmap: Vec<u8>,
    width: u16,
    height: u16,
    color_type: png::ColorType,
}

/// Computes the stride of an uncompressed RGB bitmap.
///
/// INVARIANT: `width <= output (stride) <= width * 4`
///
/// In an uncompressed bitmap, the stride is the number of bytes needed to go from the start of one
/// row of pixels to the start of the next row. The image format defines a minimum stride for an
/// image. In addition, the graphics hardware might require a larger stride for the surface that
/// contains the image.
///
/// For uncompressed RGB formats, the minimum stride is always the image width in bytes, rounded up
/// to the nearest DWORD (4 bytes). The following formula is used to calculate the stride:
///
/// ```
/// stride = ((((width * bit_count) + 31) & ~31) >> 3)
/// ```
///
/// From Microsoft doc: https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader
fn rgb_bmp_stride(width: u16, bit_count: u16) -> usize {
    debug_assert!(bit_count <= 32);

    // No side effects, because u16::MAX * 32 + 31 < u16::MAX * u16::MAX < u32::MAX
    #[expect(clippy::arithmetic_side_effects)]
    {
        (((usize::from(width) * usize::from(bit_count)) + 31) & !31) >> 3
    }
}

fn bgra_to_top_down_rgba(
    header: &BitmapInfoHeader,
    src_bitmap: &[u8],
    preserve_alpha: bool,
) -> Result<PngEncoderContext, BitmapError> {
    // DIB may be encoded bottom-up, but the format we target, PNG, is top-down.
    let should_flip_vertically = header.is_bottom_up();

    let width = header.width();
    let height = header.height();

    let src_n_samples = usize::from(header.bit_count / 8);

    let src_stride = rgb_bmp_stride(width, header.bit_count);

    let (dst_color_type, dst_n_samples) = if preserve_alpha {
        (png::ColorType::Rgba, 4)
    } else {
        (png::ColorType::Rgb, 3)
    };

    // Per invariants: height * width * dst_n_samples <= 10_000 * 10_000 * 4 < u32::MAX
    #[expect(clippy::arithmetic_side_effects)]
    let dst_bitmap_len = usize::from(height) * usize::from(width) * dst_n_samples;

    // Prevent allocation of huge buffers.
    ensure(dst_bitmap_len <= MAX_BUFFER_SIZE).ok_or(BitmapError::BufferTooBig)?;

    let mut rows_normal;
    let mut rows_reversed;

    let rows: &mut dyn Iterator<Item = &[u8]> = if should_flip_vertically {
        rows_reversed = src_bitmap.chunks_exact(src_stride).rev();
        &mut rows_reversed
    } else {
        rows_normal = src_bitmap.chunks_exact(src_stride);
        &mut rows_normal
    };

    // DIB stores BGRA colors while PNG uses RGBA.
    // DIBv1 (CF_DIB) does not have alpha channel, and the fourth byte is always set to 0xFF.
    // DIBv5 (CF_DIBV5) supports alpha channel, so we should preserve it if it is present.
    let transform: fn((&mut [u8], &[u8])) = match (header.bit_count, dst_color_type) {
        (24 | 32, png::ColorType::Rgb) => |(pixel_out, pixel_in)| {
            pixel_out[0] = pixel_in[2];
            pixel_out[1] = pixel_in[1];
            pixel_out[2] = pixel_in[0];
        },
        (24, png::ColorType::Rgba) => |(pixel_out, pixel_in)| {
            pixel_out[0] = pixel_in[2];
            pixel_out[1] = pixel_in[1];
            pixel_out[2] = pixel_in[0];
            pixel_out[3] = 0xFF;
        },
        (32, png::ColorType::Rgba) => |(pixel_out, pixel_in)| {
            pixel_out[0] = pixel_in[2];
            pixel_out[1] = pixel_in[1];
            pixel_out[2] = pixel_in[0];
            pixel_out[3] = pixel_in[3];
        },
        _ => unreachable!("possible values are restricted by header validation and logic above"),
    };

    // Per invariants: width * dst_n_samples <= 10_000 * 4 < u32::MAX
    #[expect(clippy::arithmetic_side_effects)]
    let dst_stride = usize::from(width) * dst_n_samples;

    let mut dst_bitmap = vec![0u8; dst_bitmap_len];

    dst_bitmap
        .chunks_exact_mut(dst_stride)
        .zip(rows)
        .for_each(|(dst_row, src_row)| {
            let dst_pixels = dst_row.chunks_exact_mut(dst_n_samples);
            let src_pixels = src_row.chunks_exact(src_n_samples);
            dst_pixels.zip(src_pixels).for_each(transform);
        });

    Ok(PngEncoderContext {
        bitmap: dst_bitmap,
        width,
        height,
        color_type: dst_color_type,
    })
}

fn encode_png(ctx: &PngEncoderContext) -> Result<Vec<u8>, BitmapError> {
    let mut output: Vec<u8> = Vec::new();

    let width = u32::from(ctx.width);
    let height = u32::from(ctx.height);

    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(ctx.color_type);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder.write_header()?;
    writer.write_image_data(&ctx.bitmap)?;
    writer.finish()?;

    Ok(output)
}

/// Converts `CF_DIB` to PNG.
pub fn dib_to_png(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    let mut src = ReadCursor::new(input);
    let header = BitmapInfoHeader::decode(&mut src).map_err(BitmapError::Decode)?;

    validate_v1_header(&header)?;

    // We support only uncompressed DIB bitmaps as it is the most common case for clipboard-copied bitmaps.
    // However, for DIBv1 specifically, BitmapCompression::BITFIELDS is not supported even when the order is BGRA,
    // because there is an additional variable-sized header holding the color masks that we don’t support yet.
    const DIBV1_SUPPORTED_COMPRESSION: &[BitmapCompression] = &[BitmapCompression::RGB];

    if !DIBV1_SUPPORTED_COMPRESSION.contains(&header.compression) {
        return Err(BitmapError::Unsupported("unsupported compression"));
    }

    let png_ctx = bgra_to_top_down_rgba(&header, src.remaining(), false)?;
    encode_png(&png_ctx)
}

/// Converts `CF_DIB` to PNG.
pub fn dibv5_to_png(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    let mut src = ReadCursor::new(input);
    let header = BitmapV5Header::decode(&mut src).map_err(BitmapError::Decode)?;

    validate_v5_header(&header)?;

    let png_ctx = bgra_to_top_down_rgba(&header.v1, src.remaining(), true)?;
    encode_png(&png_ctx)
}

fn top_down_rgba_to_bottom_up_bgra(
    info: png::OutputInfo,
    src_bitmap: &[u8],
) -> Result<(BitmapInfoHeader, Vec<u8>), BitmapError> {
    let no_alpha = info.color_type != png::ColorType::Rgba;
    let width = u16::try_from(info.width).map_err(|_| BitmapError::WidthTooBig)?;
    let height = u16::try_from(info.height).map_err(|_| BitmapError::HeightTooBig)?;

    #[expect(clippy::arithmetic_side_effects)] // width * 4 <= 10_000 * 4 < u32::MAX
    let stride = usize::from(width) * 4;

    let src_rows = src_bitmap.chunks_exact(stride);

    // As per invariants: stride * height <= width * 4 * height <= 10_000 * 4 * 10_000 <= u32::MAX.
    #[expect(clippy::arithmetic_side_effects)]
    let dst_len = stride * usize::from(height);
    let dst_len = u32::try_from(dst_len).map_err(|_| BitmapError::InvalidSize)?;

    let header = BitmapInfoHeader {
        width: i32::from(width),
        height: i32::from(height),
        bit_count: 32, // 4 samples * 8 bits
        compression: BitmapCompression::RGB,
        size_image: dst_len,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: 0,
        clr_important: 0,
    };

    let dst_len = usize::try_from(dst_len).map_err(|_| BitmapError::InvalidSize)?;
    let mut dst_bitmap = vec![0; dst_len];

    // Reverse rows to draw the image from bottom to top.
    let dst_rows = dst_bitmap.chunks_exact_mut(stride).rev();

    let transform: fn((&mut [u8], &[u8])) = if no_alpha {
        |(dst_pixel, src_pixel)| {
            dst_pixel[0] = src_pixel[2];
            dst_pixel[1] = src_pixel[1];
            dst_pixel[2] = src_pixel[0];
            dst_pixel[3] = 0xFF;
        }
    } else {
        |(dst_pixel, src_pixel)| {
            dst_pixel[0] = src_pixel[2];
            dst_pixel[1] = src_pixel[1];
            dst_pixel[2] = src_pixel[0];
            dst_pixel[3] = src_pixel[3];
        }
    };

    dst_rows.zip(src_rows).for_each(|(dst_row, src_row)| {
        let dst_pixels = dst_row.chunks_exact_mut(4);
        let src_pixels = src_row.chunks_exact(4);
        dst_pixels.zip(src_pixels).for_each(transform);
    });

    Ok((header, dst_bitmap))
}

fn decode_png(mut input: &[u8]) -> Result<(png::OutputInfo, Vec<u8>), BitmapError> {
    let mut decoder = png::Decoder::new(Cursor::new(&mut input));

    // We need to produce 32-bit DIB, so we should expand the palette to 32-bit RGBA.
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::EXPAND);

    let mut reader = decoder.read_info()?;
    let Some(output_buffer_len) = reader.output_buffer_size() else {
        return Err(BitmapError::BufferTooBig);
    };

    // Prevent allocation of huge buffers.
    ensure(output_buffer_len <= MAX_BUFFER_SIZE).ok_or(BitmapError::BufferTooBig)?;

    let mut buffer = vec![0; output_buffer_len];
    let info = reader.next_frame(&mut buffer)?;
    buffer.truncate(info.buffer_size());

    Ok((info, buffer))
}

/// Converts PNG to `CF_DIB` format.
pub fn png_to_cf_dib(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    // FIXME(perf): it’s possible to allocate a single array and to directly write both the header and the actual bitmap inside.
    // Currently, the code is performing three allocations: one inside `decode_png`, one inside `top_down_rgba_to_bottom_up_bgra`
    // and one in the body of this function.

    let (png_info, rgba_bytes) = decode_png(input)?;
    let (header, bgra_bytes) = top_down_rgba_to_bottom_up_bgra(png_info, &rgba_bytes)?;

    let output_len = header
        .size()
        .checked_add(bgra_bytes.len())
        .ok_or(BitmapError::BufferTooBig)?;

    ensure(output_len <= MAX_BUFFER_SIZE).ok_or(BitmapError::BufferTooBig)?;

    let mut output = vec![0; output_len];
    {
        let mut dst = WriteCursor::new(&mut output);
        header.encode(&mut dst).map_err(BitmapError::Encode)?;
        dst.write_slice(&bgra_bytes);
    }

    Ok(output)
}

/// Converts PNG to `CF_DIBV5` format.
pub fn png_to_cf_dibv5(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    // FIXME(perf): it’s possible to allocate a single array and to directly write both the header and the actual bitmap inside.
    // Currently, the code is performing three allocations: one inside `decode_png`, one inside `top_down_rgba_to_bottom_up_bgra`
    // and one in the body of this function.

    let (png_info, rgba_bytes) = decode_png(input)?;
    let (header_v1, bgra_bytes) = top_down_rgba_to_bottom_up_bgra(png_info, &rgba_bytes)?;

    let header = BitmapV5Header {
        v1: header_v1,
        // Windows sets these masks for 32-bit bitmaps even if BITFIELDS compression is not used.
        red_mask: 0x00FF0000,
        green_mask: 0x0000FF00,
        blue_mask: 0x000000FF,
        alpha_mask: 0xFF000000,
        color_space: ColorSpace::SRGB,
        endpoints: Default::default(),
        gamma_red: 0,
        gamma_green: 0,
        gamma_blue: 0,
        intent: BitmapIntent::LCS_GM_IMAGES,
        profile_data: 0,
        profile_size: 0,
    };

    let output_len = header
        .size()
        .checked_add(bgra_bytes.len())
        .ok_or(BitmapError::BufferTooBig)?;

    ensure(output_len <= MAX_BUFFER_SIZE).ok_or(BitmapError::BufferTooBig)?;

    let mut output = vec![0; output_len];
    {
        let mut dst = WriteCursor::new(&mut output);
        header.encode(&mut dst).map_err(BitmapError::Encode)?;
        dst.write_slice(&bgra_bytes);
    }

    Ok(output)
}

/// Use this when establishing invariants.
#[inline]
#[must_use]
fn check_invariant(condition: bool) -> Option<()> {
    condition.then_some(())
}

/// Returns `None` when the condition is unmet.
#[inline]
#[must_use]
fn ensure(condition: bool) -> Option<()> {
    condition.then_some(())
}
