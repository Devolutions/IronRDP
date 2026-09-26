//! PNG <-> RGBA conversion for the OS clipboard.
//!
//! `arboard` hands images over as tightly packed 8-bit RGBA, while
//! `ironrdp-cliprdr-format` converts between `CF_DIB`/`CF_DIBV5` and PNG, so
//! PNG is the meeting point between the two.

use core::fmt;

/// A tightly packed 8-bit RGBA image, row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rgba {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(super) enum ImageError {
    Decode(png::DecodingError),
    Encode(png::EncodingError),
    /// The image dimensions and the pixel buffer disagree, or the image is too large to hold.
    Size,
    UnsupportedColorType(png::ColorType),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "PNG decoding failed: {error}"),
            Self::Encode(error) => write!(f, "PNG encoding failed: {error}"),
            Self::Size => f.write_str("image dimensions do not match the pixel buffer"),
            Self::UnsupportedColorType(color_type) => write!(f, "unsupported PNG color type {color_type:?}"),
        }
    }
}

impl core::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Size | Self::UnsupportedColorType(_) => None,
        }
    }
}

impl From<png::DecodingError> for ImageError {
    fn from(error: png::DecodingError) -> Self {
        Self::Decode(error)
    }
}

impl From<png::EncodingError> for ImageError {
    fn from(error: png::EncodingError) -> Self {
        Self::Encode(error)
    }
}

/// Decodes a PNG into 8-bit RGBA, expanding palettes, grayscale and 16-bit
/// samples along the way.
pub(super) fn png_to_rgba(png: &[u8]) -> Result<Rgba, ImageError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let size = reader.output_buffer_size().ok_or(ImageError::Size)?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer)?;
    buffer.truncate(info.buffer_size());

    let bytes = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 0xFF])
            .collect::<Vec<u8>>(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|&g| [g, g, g, 0xFF]).collect::<Vec<u8>>(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .flat_map(|px| [px[0], px[0], px[0], px[1]])
            .collect::<Vec<u8>>(),
        other @ png::ColorType::Indexed => return Err(ImageError::UnsupportedColorType(other)),
    };

    let expected = usize::try_from(info.width)
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(info.height).ok()?))
        .and_then(|px| px.checked_mul(4))
        .ok_or(ImageError::Size)?;
    if bytes.len() != expected {
        return Err(ImageError::Size);
    }

    Ok(Rgba {
        width: info.width,
        height: info.height,
        bytes,
    })
}

/// Encodes 8-bit RGBA as a PNG.
pub(super) fn rgba_to_png(image: &Rgba) -> Result<Vec<u8>, ImageError> {
    let expected = usize::try_from(image.width)
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(image.height).ok()?))
        .and_then(|px| px.checked_mul(4))
        .ok_or(ImageError::Size)?;
    if image.bytes.len() != expected || image.width == 0 || image.height == 0 {
        return Err(ImageError::Size);
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&image.bytes)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Rgba {
        Rgba {
            width: 2,
            height: 2,
            bytes: vec![
                0xFF, 0x00, 0x00, 0xFF, // red
                0x00, 0xFF, 0x00, 0x80, // half-transparent green
                0x00, 0x00, 0xFF, 0xFF, // blue
                0x10, 0x20, 0x30, 0x00, // transparent
            ],
        }
    }

    #[test]
    fn rgba_survives_a_png_round_trip() {
        let image = sample();
        let png = rgba_to_png(&image).expect("encode");
        assert_eq!(png_to_rgba(&png).expect("decode"), image);
    }

    #[test]
    fn rgb_png_gets_an_opaque_alpha_channel() {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&[1, 2, 3]).expect("data");
        }
        let image = png_to_rgba(&out).expect("decode");
        assert_eq!(image.bytes, vec![1, 2, 3, 0xFF]);
    }

    #[test]
    fn mismatched_dimensions_are_rejected() {
        let image = Rgba {
            width: 3,
            height: 1,
            bytes: vec![0; 8],
        };
        assert!(matches!(rgba_to_png(&image), Err(ImageError::Size)));
    }
}
