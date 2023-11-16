//! This module implements logic to decode pointer PDUs into RGBA bitmaps ready for rendering.
//!
//! #### References:
//! - Drawing pointers: https://learn.microsoft.com/en-us/windows-hardware/drivers/display/pointer-drawing
//! - Drawing color pointers: https://learn.microsoft.com/en-us/windows-hardware/drivers/display/drawing-color-pointers
//! -Drawing monochrome pointers https://learn.microsoft.com/en-us/windows-hardware/drivers/display/drawing-monochrome-pointers
//!
//!
//! #### Notes on xor/and masks encoding:
//! RDP's pointer representation is a bit weird. It uses two masks to represent a pointer -
//! andMask and xorMask. Xor mask is used as a base color for a pointer pixel, and andMask
//! mask is used co control pixel's full transparency (`src_color.a = 0`), full opacity
//! (`src_color.a = 255`) or pixel inversion (`dst_color.rgb = vec3(255) - dst_color.rgb`).
//!
//! Xor basks could be 1, 8, 16, 24 or 32 bits per pixel, and andMask is always 1 bit per pixel.
//!
//! Rules for decoding masks:
//! - `andMask == 0` -> dst_color Copy pixel from xorMask
//! - andMask == 1, xorMask == 0(black color) -> Transparent pixel
//! - andMask == 1, xorMask == 1(white color) -> Pixel is inverted

use ironrdp_pdu::cursor::ReadCursor;
use ironrdp_pdu::pointer::{ColorPointerAttribute, LargePointerAttribute, PointerAttribute};
use thiserror::Error;

use crate::color_conversion::rdp_16bit_to_rgb;

#[derive(Debug, Error)]
pub enum PointerError {
    #[error("invalid pointer xorMask size. Expected: {expected}, actual: {actual}")]
    InvalidXorMaskSize { expected: usize, actual: usize },
    #[error("invalid pointer andMask size. Expected: {expected}, actual: {actual}")]
    InvalidAndMaskSize { expected: usize, actual: usize },
    #[error("not supported pointer bpp: {bpp}")]
    NotSupportedBpp { bpp: usize },
    #[error(transparent)]
    Pdu(#[from] ironrdp_pdu::PduError),
}

/// Represents RDP pointer in decoded form (color channels stored as RGBA pre-multiplied values)
#[derive(Debug)]
pub struct DecodedPointer {
    pub width: usize,
    pub height: usize,
    pub hot_spot_x: usize,
    pub hot_spot_y: usize,
    pub bitmap_data: Vec<u8>,
}

impl DecodedPointer {
    pub fn new_invisible() -> Self {
        Self {
            width: 0,
            height: 0,
            bitmap_data: Vec::new(),
            hot_spot_x: 0,
            hot_spot_y: 0,
        }
    }

    pub fn decode_pointer_attribute(src: &PointerAttribute<'_>) -> Result<Self, PointerError> {
        Self::decode_pointer(PointerData {
            width: src.color_pointer.width as usize,
            height: src.color_pointer.height as usize,
            xor_bpp: src.xor_bpp as usize,
            xor_mask: src.color_pointer.xor_mask,
            and_mask: src.color_pointer.and_mask,
            hot_spot_x: src.color_pointer.hot_spot.x as usize,
            hot_spot_y: src.color_pointer.hot_spot.y as usize,
        })
    }

    pub fn decode_color_pointer_attribute(src: &ColorPointerAttribute<'_>) -> Result<Self, PointerError> {
        Self::decode_pointer(PointerData {
            width: src.width as usize,
            height: src.height as usize,
            xor_bpp: 24,
            xor_mask: src.xor_mask,
            and_mask: src.and_mask,
            hot_spot_x: src.hot_spot.x as usize,
            hot_spot_y: src.hot_spot.y as usize,
        })
    }

    pub fn decode_large_pointer_attribute(src: &LargePointerAttribute<'_>) -> Result<Self, PointerError> {
        Self::decode_pointer(PointerData {
            width: src.width as usize,
            height: src.height as usize,
            xor_bpp: src.xor_bpp as usize,
            xor_mask: src.xor_mask,
            and_mask: src.and_mask,
            hot_spot_x: src.hot_spot.x as usize,
            hot_spot_y: src.hot_spot.y as usize,
        })
    }

    fn decode_pointer(data: PointerData<'_>) -> Result<Self, PointerError> {
        const SUPPORTED_COLOR_BPP: [usize; 4] = [1, 16, 24, 32];

        if data.width == 0 || data.height == 0 {
            return Ok(Self::new_invisible());
        }

        if !SUPPORTED_COLOR_BPP.contains(&data.xor_bpp) {
            // 8bpp indexed colors are not supported yet (palette messages are not implemented)
            // Other unknown bpps are not supported either
            return Err(PointerError::NotSupportedBpp { bpp: data.xor_bpp });
        }

        let flip_vertical = data.xor_bpp != 1;

        let and_stride = Stride::from_bits(data.width);
        let xor_stride = Stride::from_bits(data.width * data.xor_bpp);

        if data.xor_mask.len() != xor_stride.length * data.height {
            return Err(PointerError::InvalidXorMaskSize {
                expected: xor_stride.length * data.height,
                actual: data.xor_mask.len(),
            });
        }

        if data.and_mask.len() != and_stride.length * data.height {
            return Err(PointerError::InvalidAndMaskSize {
                expected: and_stride.length * data.height,
                actual: data.and_mask.len(),
            });
        }

        let mut bitmap_data = Vec::new();

        for row_idx in 0..data.height {
            // For non-monochrome cursors we read strides from bottom to top
            let (mut xor_stride_cursor, mut and_stride_cursor) = if flip_vertical {
                let xor_stride_cursor =
                    ReadCursor::new(&data.xor_mask[(data.height - row_idx - 1) * xor_stride.length..]);
                let and_stride_cursor =
                    ReadCursor::new(&data.and_mask[(data.height - row_idx - 1) * and_stride.length..]);
                (xor_stride_cursor, and_stride_cursor)
            } else {
                let xor_stride_cursor = ReadCursor::new(&data.xor_mask[row_idx * xor_stride.length..]);
                let and_stride_cursor = ReadCursor::new(&data.and_mask[row_idx * and_stride.length..]);
                (xor_stride_cursor, and_stride_cursor)
            };

            let mut color_reader = ColorStrideReader::new(data.xor_bpp, xor_stride);
            let mut bitmask_reader = BitmaskStrideReader::new(and_stride);

            for _ in 0..data.width {
                let and_bit = bitmask_reader.next_bit(&mut and_stride_cursor);
                let color = color_reader.next_pixel(&mut xor_stride_cursor);

                if and_bit == 1 && color == [0, 0, 0, 0xff] {
                    // Force transparent pixel (The only way to get a transparent pixel with
                    // non-32-bit cursors)
                    bitmap_data.extend_from_slice(&[0, 0, 0, 0]);
                } else if and_bit == 1 && color == [0xff, 0xff, 0xff, 0xff] {
                    // Inverted pixel.

                    // Colors with alpha channel set to 0x00 are always invisible no matter their color
                    // component. We could take advantage of that, and use a special color to represent
                    // inverted pixels. For example, we could use [0xFF, 0xFF, 0xFF, 0x00] for such
                    // purpose.
                    bitmap_data.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);
                } else {
                    // Calculate premultiplied alpha via integer arithmetic
                    let with_premultiplied_alpha = [
                        ((color[0] as u16 * color[0] as u16) >> 8) as u8,
                        ((color[1] as u16 * color[1] as u16) >> 8) as u8,
                        ((color[2] as u16 * color[2] as u16) >> 8) as u8,
                        color[3],
                    ];
                    bitmap_data.extend_from_slice(&with_premultiplied_alpha);
                }
            }
        }

        Ok(Self {
            width: data.width,
            height: data.height,
            bitmap_data,
            hot_spot_x: data.hot_spot_x,
            hot_spot_y: data.hot_spot_y,
        })
    }
}

#[derive(Clone, Copy)]
struct Stride {
    length: usize,
    data_bytes: usize,
    padding: usize,
}

impl Stride {
    fn from_bits(bits: usize) -> Stride {
        let length = bit_stride_size_align_u16(bits);
        let data_bytes = bit_stride_size_align_u8(bits);
        Stride {
            length,
            data_bytes,
            padding: length - data_bytes,
        }
    }
}

struct BitmaskStrideReader {
    current_byte: u8,
    read_bits: usize,
    read_stide_bytes: usize,
    stride_data_bytes: usize,
    stride_padding: usize,
}

impl BitmaskStrideReader {
    fn new(stride: Stride) -> Self {
        Self {
            current_byte: 0,
            read_bits: 8,
            read_stide_bytes: 0,
            stride_data_bytes: stride.data_bytes,
            stride_padding: stride.padding,
        }
    }

    fn next_bit(&mut self, cursor: &mut ReadCursor<'_>) -> u8 {
        if self.read_bits == 8 {
            self.read_bits = 0;

            if self.read_stide_bytes == self.stride_data_bytes {
                self.read_stide_bytes = 0;
                cursor.read_slice(self.stride_padding);
            }

            self.current_byte = cursor.read_u8();
        }

        let bit = (self.current_byte >> (7 - self.read_bits)) & 1;
        self.read_bits += 1;
        bit
    }
}

enum ColorStrideReader {
    Color {
        bpp: usize,
        read_stide_bytes: usize,
        stride_data_bytes: usize,
        stride_padding: usize,
    },
    Bitmask(BitmaskStrideReader),
}

impl ColorStrideReader {
    fn new(bpp: usize, stride: Stride) -> Self {
        match bpp {
            1 => Self::Bitmask(BitmaskStrideReader::new(stride)),
            bpp => Self::Color {
                bpp,
                read_stide_bytes: 0,
                stride_data_bytes: stride.data_bytes,
                stride_padding: stride.padding,
            },
        }
    }

    fn next_pixel(&mut self, cursor: &mut ReadCursor<'_>) -> [u8; 4] {
        match self {
            ColorStrideReader::Color {
                bpp,
                read_stide_bytes,
                stride_data_bytes,
                stride_padding,
            } => {
                if read_stide_bytes == stride_data_bytes {
                    *read_stide_bytes = 0;
                    cursor.read_slice(*stride_padding);
                }

                match bpp {
                    16 => {
                        *read_stide_bytes += 2;
                        let color_16bit = cursor.read_u16();
                        let [r, g, b] = rdp_16bit_to_rgb(color_16bit);
                        [r, g, b, 0xff]
                    }
                    24 => {
                        *read_stide_bytes += 3;

                        let color_24bit = cursor.read_array::<3>();
                        [color_24bit[2], color_24bit[1], color_24bit[0], 0xff]
                    }
                    32 => {
                        *read_stide_bytes += 4;
                        let color_32bit = cursor.read_array::<4>();
                        [color_32bit[2], color_32bit[1], color_32bit[0], color_32bit[3]]
                    }
                    _ => panic!("BUG: should be validated in the calling code"),
                }
            }
            ColorStrideReader::Bitmask(bitask) => {
                if bitask.next_bit(cursor) == 1 {
                    [0xff, 0xff, 0xff, 0xff]
                } else {
                    [0, 0, 0, 0xff]
                }
            }
        }
    }
}

fn bit_stride_size_align_u8(size_bits: usize) -> usize {
    (size_bits + 7) / 8
}

fn bit_stride_size_align_u16(size_bits: usize) -> usize {
    ((size_bits + 15) / 16) * 2
}

/// Message-agnostic pointer data.
struct PointerData<'a> {
    width: usize,
    height: usize,
    xor_bpp: usize,
    xor_mask: &'a [u8],
    and_mask: &'a [u8],
    hot_spot_x: usize,
    hot_spot_y: usize,
}
