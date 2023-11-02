use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{cast_int, ensure_fixed_part_size, invalid_message_err, PduDecode, PduEncode, PduResult};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BitmapError {
    #[error("Invalid bitmap header")]
    InvalidHeader(ironrdp_pdu::PduError),
    #[error("Unsupported bitmap: {0}")]
    Unsupported(&'static str),
    #[error("One of bitmap's dimensions is invalid")]
    InvalidSize,
    #[error("PNG encoding error")]
    PngEncode(#[from] png::EncodingError),
    #[error("PNG decoding error")]
    PngDecode(#[from] png::DecodingError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitmapCompression(pub u32);

impl BitmapCompression {
    pub const RGB: Self = Self(0x0000);
    pub const RLE8: Self = Self(0x0001);
    pub const RLE4: Self = Self(0x0002);
    pub const BITFIELDS: Self = Self(0x0003);
    pub const JPEG: Self = Self(0x0004);
    pub const PNG: Self = Self(0x0005);
    pub const CMYK: Self = Self(0x000B);
    pub const CMYKRLE8: Self = Self(0x000C);
    pub const CMYKRLE4: Self = Self(0x000D);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorSpace(pub u32);

impl ColorSpace {
    pub const CALIBRATED_RGB: Self = Self(0x00000000);
    pub const SRGB: Self = Self(0x73524742);
    pub const WINDOWS: Self = Self(0x57696E20);
    pub const PROFILE_LINKED: Self = Self(0x4C494E4B);
    pub const PROFILE_EMBEDDED: Self = Self(0x4D424544);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitmapIntent(pub u32);

impl BitmapIntent {
    pub const LCS_GM_ABS_COLORIMETRIC: Self = Self(0x00000008);
    pub const LCS_GM_BUSINESS: Self = Self(0x00000001);
    pub const LCS_GM_GRAPHICS: Self = Self(0x00000002);
    pub const LCS_GM_IMAGES: Self = Self(0x00000004);
}

pub type Fxpt2Dot30 = u32; // (LONG)

#[derive(Default)]
pub struct Ciexyz {
    pub x: Fxpt2Dot30,
    pub y: Fxpt2Dot30,
    pub z: Fxpt2Dot30,
}

#[derive(Default)]
pub struct CiexyzTriple {
    pub red: Ciexyz,
    pub green: Ciexyz,
    pub blue: Ciexyz,
}

impl CiexyzTriple {
    const NAME: &str = "CIEXYZTRIPLE";
    const FIXED_PART_SIZE: usize = 4 * 3 * 3; // 4(LONG) * 3(xyz) * 3(red, green, blue)
}

impl<'a> PduDecode<'a> for CiexyzTriple {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
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

impl PduEncode for CiexyzTriple {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapInfoHeader {
    pub width: i32,
    pub height: i32,
    pub bit_count: u16,
    pub compression: BitmapCompression,
    pub size_image: u32,
    pub x_pels_per_meter: i32,
    pub y_pels_per_meter: i32,
    pub clr_used: u32,
    pub clr_important: u32,
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

    const NAME: &str = "BITMAPINFOHEADER";

    pub fn has_color_table(&self) -> bool {
        (self.bit_count <= 8) && (self.compression == BitmapCompression::RGB)
    }

    fn encode_with_size(&self, dst: &mut WriteCursor<'_>, size: u32) -> PduResult<()> {
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

    fn decode_with_size(src: &mut ReadCursor<'_>) -> PduResult<(Self, u32)> {
        ensure_fixed_part_size!(in: src);

        let size = src.read_u32();

        let width = src.read_i32();
        let height = src.read_i32();
        let planes = src.read_u16();
        if planes != 1 {
            return Err(invalid_message_err!("biPlanes", "invalid planes count"));
        }
        let bit_count = src.read_u16();
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
}

impl PduEncode for BitmapInfoHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

impl<'a> PduDecode<'a> for BitmapInfoHeader {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        let (header, size) = Self::decode_with_size(src)?;
        let size: usize = cast_int!("biSize", size)?;

        if size != Self::FIXED_PART_SIZE {
            return Err(invalid_message_err!("biSize", "invalid V1 bitmap info header size"));
        }

        Ok(header)
    }
}

pub struct BitmapV5Header {
    pub header_v1: BitmapInfoHeader,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub alpha_mask: u32,
    pub color_space: ColorSpace,
    pub endpoints: CiexyzTriple,
    pub gamma_red: u32,
    pub gamma_green: u32,
    pub gamma_blue: u32,
    pub intent: BitmapIntent,
    pub profile_data: u32,
    pub profile_size: u32,
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

    const NAME: &str = "BITMAPV5HEADER";
}

impl PduEncode for BitmapV5Header {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let size = cast_int!("biSize", Self::FIXED_PART_SIZE)?;
        self.header_v1.encode_with_size(dst, size)?;

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

impl<'a> PduDecode<'a> for BitmapV5Header {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let (header_v1, size) = BitmapInfoHeader::decode_with_size(src)?;
        let size: usize = cast_int!("biSize", size)?;

        if size != Self::FIXED_PART_SIZE {
            return Err(invalid_message_err!("biSize", "invalid V5 bitmap info header size"));
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
            header_v1,
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

    // We support only uncompressed DIB bitmaps as it is the most common case for clipboard-copied
    // bitmaps.
    const SUPPORTED_COMPRESSION: &[BitmapCompression] = &[BitmapCompression::RGB];
    if !SUPPORTED_COMPRESSION.contains(&header.compression) {
        return Err(BitmapError::Unsupported("unsupported compression"));
    }

    // This is only relevant for bitmaps with bpp < 24, which are not supported.
    if header.clr_used != 0 {
        return Err(BitmapError::Unsupported("color table is not supported"));
    }

    Ok(())
}

fn validate_v5_header(header: &BitmapV5Header) -> Result<(), BitmapError> {
    validate_v1_header(&header.header_v1)?;

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

struct PngEncoderInput {
    frame_buffer: Vec<u8>,
    width: usize,
    height: usize,
    color_type: png::ColorType,
}

/// From MS docs:
/// For uncompressed RGB formats, the minimum stride is always the image width in bytes, rounded
/// up to the nearest DWORD. You can use the following formula to calculate the stride and image
/// size:
/// ```
/// stride = ((((biWidth * biBitCount) + 31) & ~31) >> 3);
/// biSizeImage = abs(biHeight) * stride;
/// ```
fn bmp_stride(width: usize, bit_count: usize) -> usize {
    (((width * bit_count) + 31) & !31) >> 3
}

fn transform_bitmap(
    header: &BitmapInfoHeader,
    input: &[u8],
    preserve_alpha: bool,
) -> Result<PngEncoderInput, BitmapError> {
    // If height is positive, DIB is bottom-up, but target PNG format is top-down.
    let flip = header.height > 0;

    let width: usize = cast_int!(BitmapInfoHeader::NAME, "biWidth", header.width).expect("width is positive");
    let height: usize = cast_int!(BitmapInfoHeader::NAME, "biHeight", header.height.abs()).expect("height is positive");

    let bit_count = usize::from(header.bit_count);

    let stride = bmp_stride(width, bit_count);

    let input_bytes_per_pixel = bit_count / 8;
    let color_type = if preserve_alpha {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    };

    let components = color_type.samples();

    let mut frame_buffer = vec![0u8; height * width * components];

    let mut strides_normal;
    let mut strides_reversed;

    let strides: &mut dyn Iterator<Item = &[u8]> = if flip {
        strides_reversed = input.chunks_exact(stride).rev();
        &mut strides_reversed
    } else {
        strides_normal = input.chunks_exact(stride);
        &mut strides_normal
    };

    // DIB stores color as strided BGRA, PNG require packed RGBA. DIBv1 (CF_DIB) do not have alpha,
    // and the fourth byte is always set to 0xFF. DIBv5 (CF_DIBV5) may have alpha, so we should
    // preserve it if it is present.
    let transform: fn((&mut [u8], &[u8])) = match (header.bit_count, color_type) {
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

    frame_buffer
        .chunks_exact_mut(width * components)
        .zip(strides)
        .for_each(|(row, stride)| {
            let input = stride.chunks_exact(input_bytes_per_pixel);
            row.chunks_exact_mut(components).zip(input).for_each(transform);
        });

    Ok(PngEncoderInput {
        frame_buffer,
        width,
        height,
        color_type,
    })
}

fn encode_png(input: PngEncoderInput) -> Result<Vec<u8>, BitmapError> {
    let mut output: Vec<u8> = Vec::new();

    let width: u32 = cast_int!("PNG encode", "width", input.width).unwrap();
    let height: u32 = cast_int!("PNG encode", "height", input.height).unwrap();

    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(input.color_type);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder.write_header()?;
    writer.write_image_data(&input.frame_buffer)?;
    writer.finish()?;

    Ok(output)
}

pub fn dib_to_png(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    let mut src = ReadCursor::new(input);
    let header = BitmapInfoHeader::decode(&mut src).map_err(BitmapError::InvalidHeader)?;

    validate_v1_header(&header)?;

    let png_inputs = transform_bitmap(&header, src.remaining(), false)?;
    encode_png(png_inputs)
}

pub fn dibv5_to_png(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    let mut src = ReadCursor::new(input);
    let header = BitmapV5Header::decode(&mut src).map_err(BitmapError::InvalidHeader)?;

    validate_v5_header(&header)?;

    let png_inputs = transform_bitmap(&header.header_v1, src.remaining(), true)?;
    encode_png(png_inputs)
}

pub fn transform_png(info: png::OutputInfo, input_buffer: Vec<u8>) -> Result<(BitmapInfoHeader, Vec<u8>), BitmapError> {
    let no_alpha = info.color_type != png::ColorType::Rgba;

    let stride = bmp_stride(
        cast_int!("BMP stride", "biWidth", info.width).map_err(|_| BitmapError::InvalidSize)?,
        32,
    );

    let width_unsigned: usize = info.width.try_into().map_err(|_| BitmapError::InvalidSize)?;
    let height_unsigned: usize = info.height.try_into().map_err(|_| BitmapError::InvalidSize)?;

    let image_size: usize = stride * height_unsigned;

    let header = BitmapInfoHeader {
        width: cast_int!("DIB header", "biWidth", info.width).map_err(|_| BitmapError::InvalidSize)?,
        height: cast_int!("DIB header", "biHeight", info.height).map_err(|_| BitmapError::InvalidSize)?,
        bit_count: 32,
        compression: BitmapCompression::RGB,
        size_image: cast_int!("DIB header", "biImageSize", image_size).map_err(|_| BitmapError::InvalidSize)?,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: 0,
        clr_important: 0,
    };

    // Row is in RGBA format
    let row_size: usize = 4 * width_unsigned;

    let mut output_buffer = vec![0; image_size];

    let rows = input_buffer.chunks_exact(row_size);

    // Reverse strides to draw image bottom-up
    let strides = output_buffer.chunks_exact_mut(stride).rev();

    let transform: fn((&mut [u8], &[u8])) = if no_alpha {
        |(pixel_out, pixel_in)| {
            pixel_out[0] = pixel_in[2];
            pixel_out[1] = pixel_in[1];
            pixel_out[2] = pixel_in[0];
            pixel_out[3] = 0xFF;
        }
    } else {
        |(pixel_out, pixel_in)| {
            pixel_out[0] = pixel_in[2];
            pixel_out[1] = pixel_in[1];
            pixel_out[2] = pixel_in[0];
            pixel_out[3] = pixel_in[3];
        }
    };

    strides.zip(rows).for_each(|(output, input)| {
        let input = input.chunks_exact(4);
        output.chunks_exact_mut(4).zip(input).for_each(transform);
    });

    Ok((header, output_buffer))
}

fn decode_png(mut input: &[u8]) -> Result<(png::OutputInfo, Vec<u8>), BitmapError> {
    let mut decoder = png::Decoder::new(&mut input);

    // We need to produce 32-bit DIB, so we should expand the palette to 32-bit RGBA.
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::EXPAND);

    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer)?;
    buffer.truncate(info.buffer_size());

    Ok((info, buffer))
}

pub fn png_to_cf_dib(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    let (info, input_buffer) = decode_png(input)?;
    let (header, output_buffer) = transform_png(info, input_buffer)?;

    let mut dib_buffer = vec![0; header.size() + output_buffer.len()];
    {
        let mut dst = WriteCursor::new(&mut dib_buffer);
        header.encode(&mut dst).map_err(BitmapError::InvalidHeader)?;
        dst.write_slice(&output_buffer);
    }

    Ok(dib_buffer)
}

pub fn png_to_cf_dibv5(input: &[u8]) -> Result<Vec<u8>, BitmapError> {
    let (info, input_buffer) = decode_png(input)?;
    let (header_v1, output_buffer) = transform_png(info, input_buffer)?;

    let header = BitmapV5Header {
        header_v1,
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

    let mut dib_buffer: Vec<u8> = vec![0; header.size() + output_buffer.len()];
    {
        let mut dst = WriteCursor::new(&mut dib_buffer);
        header.encode(&mut dst).map_err(BitmapError::InvalidHeader)?;
        dst.write_slice(&output_buffer);
    }

    Ok(dib_buffer)
}
