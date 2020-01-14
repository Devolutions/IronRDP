mod circular_buffer;
mod control_messages;
#[cfg(test)]
mod tests;

use std::{
    io::{self, Write},
    ops,
};

use bitvec::prelude::{BitSlice, BitVec, Msb0 as BigEndian};
use byteorder::{ReadBytesExt, WriteBytesExt};
use failure::Fail;
use lazy_static::lazy_static;

use crate::impl_from_error;
use circular_buffer::FixedCircularBuffer;
use control_messages::{BulkEncodedData, CompressionFlags, SegmentedDataPdu};

const HISTORY_SIZE: usize = 2_500_000;

pub struct Decompressor {
    history: FixedCircularBuffer,
}

impl Decompressor {
    pub fn new() -> Self {
        Self {
            history: FixedCircularBuffer::new(HISTORY_SIZE),
        }
    }

    pub fn decompress(
        &mut self,
        input: &[u8],
        mut output: &mut Vec<u8>,
    ) -> Result<usize, ZgfxError> {
        let segmented_data = SegmentedDataPdu::from_buffer(input)?;

        match segmented_data {
            SegmentedDataPdu::Single(segment) => self.handle_segment(&segment, &mut output),
            SegmentedDataPdu::Multipart {
                uncompressed_size,
                segments,
            } => {
                let mut bytes_written = 0;
                for segment in segments {
                    let written = self.handle_segment(&segment, &mut output)?;
                    bytes_written += written;
                }

                if bytes_written != uncompressed_size {
                    Err(ZgfxError::InvalidDecompressedSize {
                        decompressed_size: bytes_written,
                        uncompressed_size,
                    })
                } else {
                    Ok(bytes_written)
                }
            }
        }
    }

    fn handle_segment(
        &mut self,
        segment: &BulkEncodedData<'_>,
        mut output: &mut Vec<u8>,
    ) -> Result<usize, ZgfxError> {
        if !segment.data.is_empty() {
            if segment
                .compression_flags
                .contains(CompressionFlags::COMPRESSED)
            {
                self.decompress_segment(segment.data, &mut output)
            } else {
                self.history.write_all(segment.data)?;
                output.extend_from_slice(segment.data);

                Ok(segment.data.len())
            }
        } else {
            Ok(0)
        }
    }

    fn decompress_segment(
        &mut self,
        encoded_data: &[u8],
        mut output: &mut Vec<u8>,
    ) -> Result<usize, ZgfxError> {
        let mut bits = BitSlice::from_slice(encoded_data);

        // The value of the last byte indicates the number of unused bits in the final byte
        bits = &bits[..8 * (encoded_data.len() - 1) - *encoded_data.last().unwrap() as usize];
        let mut bits = Bits::new(&bits);
        let mut bytes_written = 0;

        while !bits.is_empty() {
            let token = TOKEN_TABLE
                .iter()
                .find(|token| token.prefix == bits[..token.prefix.len()])
                .ok_or_else(|| ZgfxError::TokenBitsNotFound)?;
            let _prefix = bits.split_to(token.prefix.len());

            match token.ty {
                TokenType::NullLiteral => {
                    // The prefix value is encoded with a "0" prefix,
                    // then read 8 bits containing the byte to output.
                    let value = bits.split_to(8);
                    let value = *value
                        .to_vec()
                        .into_vec()
                        .first()
                        .expect("8 bits must equal to vec of 1 byte");

                    self.history.write_u8(value)?;
                    output.push(value);
                    bytes_written += 1;
                }
                TokenType::Literal { literal_value } => {
                    self.history
                        .write_u8(literal_value)
                        .expect("circular buffer does not fail");
                    output.push(literal_value);
                    bytes_written += 1;
                }
                TokenType::Match {
                    distance_value_size,
                    distance_base,
                } => {
                    let written = handle_match(
                        &mut bits,
                        distance_value_size,
                        distance_base,
                        &mut self.history,
                        &mut output,
                    )?;
                    bytes_written += written;
                }
            }
        }

        Ok(bytes_written)
    }
}

impl Default for Decompressor {
    fn default() -> Self {
        Self::new()
    }
}

struct Bits<'a> {
    bits_slice: &'a BitSlice<BigEndian, u8>,
    remaining_bits_of_last_byte: usize,
}

impl<'a> Bits<'a> {
    pub fn new(bits_slice: &'a BitSlice<BigEndian, u8>) -> Self {
        Self {
            bits_slice,
            remaining_bits_of_last_byte: 0,
        }
    }
    pub fn split_to(&mut self, at: usize) -> &'a BitSlice<BigEndian, u8> {
        let (value, new_bits) = self.bits_slice.split_at(at);
        self.bits_slice = new_bits;
        self.remaining_bits_of_last_byte = (self.remaining_bits_of_last_byte + at) % 8;

        value
    }
    pub fn remaining_bits_of_last_byte(&self) -> usize {
        self.remaining_bits_of_last_byte
    }
}

impl<'a> ops::Deref for Bits<'a> {
    type Target = BitSlice<BigEndian, u8>;

    fn deref(&self) -> &Self::Target {
        self.bits_slice
    }
}

fn handle_match(
    bits: &mut Bits<'_>,
    distance_value_size: usize,
    distance_base: u32,
    history: &mut FixedCircularBuffer,
    mut output: &mut Vec<u8>,
) -> io::Result<usize> {
    // Each token has been assigned a different base distance
    // and number of additional value bits to be added to compute the full distance.

    let distance_value = bits.split_to(distance_value_size);
    let distance =
        (distance_base + bytes_to_u32_be(complete_to_u32(distance_value).into_vec())) as usize;

    if distance == 0 {
        read_unencoded_bytes(bits, history, &mut output)
    } else {
        read_encoded_bytes(bits, distance, history, &mut output)
    }
}

fn read_unencoded_bytes(
    bits: &mut Bits<'_>,
    history: &mut FixedCircularBuffer,
    output: &mut Vec<u8>,
) -> io::Result<usize> {
    // A match distance of zero is a special case,
    // which indicates that an unencoded run of bytes follows.
    // The count of bytes is encoded as a 15-bit value
    let length_value = bits.split_to(15);
    let length = bytes_to_u32_be(complete_to_u32(length_value).into_vec()) as usize;

    if bits.remaining_bits_of_last_byte() > 0 {
        let pad_to_byte_boundary = 8 - bits.remaining_bits_of_last_byte();
        bits.split_to(pad_to_byte_boundary);
    }

    let unencoded_bits = bits.split_to(length * 8);

    let unencoded_bytes = unencoded_bits.as_slice();
    history.write_all(unencoded_bytes)?;
    output.extend_from_slice(unencoded_bytes);

    Ok(unencoded_bytes.len())
}

fn read_encoded_bytes(
    bits: &mut Bits<'_>,
    distance: usize,
    history: &mut FixedCircularBuffer,
    mut output: &mut Vec<u8>,
) -> io::Result<usize> {
    // A match length prefix follows the token and indicates
    // how many additional bits will be needed to get the full length
    // (the number of bytes to be copied).

    let length_token_size = bits.iter().take_while(|&&v| v).map(|_| 1).sum::<usize>();
    bits.split_to(length_token_size + 1); // length token + zero bit

    let length = if length_token_size == 0 {
        // special case

        3
    } else {
        let length_value = bits.split_to(length_token_size + 1);
        let length = bytes_to_u32_be(complete_to_u32(length_value).into_vec()) as usize;

        let base = 2u32.pow(length_token_size as u32 + 1) as usize;

        base + length
    };

    let output_length = output.len();
    history.read_with_offset(distance, length, &mut output)?;
    history
        .write_all(&output[output_length..])
        .expect("circular buffer does not fail");

    Ok(length)
}

fn complete_to_u32(bits: &BitSlice<BigEndian, u8>) -> BitVec<BigEndian, u8> {
    let mut result = BitVec::<BigEndian, u8>::from_vec(vec![0; 4]);
    let result_slice = &mut result[4 * 8 - bits.len()..];
    result_slice.clone_from_slice(bits);

    result
}

fn bytes_to_u32_be(mut bytes: Vec<u8>) -> u32 {
    bytes.resize(4, 0);

    bytes
        .as_slice()
        .read_u32::<byteorder::BigEndian>()
        .expect("previously resized to 4 bytes must not fail")
}

struct Token {
    pub prefix: BitVec<BigEndian, u8>,
    pub ty: TokenType,
}

enum TokenType {
    NullLiteral,
    Literal {
        literal_value: u8,
    },
    Match {
        distance_value_size: usize,
        distance_base: u32,
    },
}

lazy_static! {
    static ref TOKEN_TABLE: [Token; 40] = [
        Token {
            prefix: slice_to_bitvec(&[0]),
            ty: TokenType::NullLiteral,
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 0, 0, 0]),
            ty: TokenType::Literal {
                literal_value: 0x00
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 0, 0, 1]),
            ty: TokenType::Literal {
                literal_value: 0x01
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 0, 1, 0, 0]),
            ty: TokenType::Literal {
                literal_value: 0x02
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 0, 1, 0, 1]),
            ty: TokenType::Literal {
                literal_value: 0x03
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 0, 1, 1, 0]),
            ty: TokenType::Literal {
                literal_value: 0x0ff
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 0, 1, 1, 1, 0]),
            ty: TokenType::Literal {
                literal_value: 0x04
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 0, 1, 1, 1, 1]),
            ty: TokenType::Literal {
                literal_value: 0x05
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 0, 0, 0]),
            ty: TokenType::Literal {
                literal_value: 0x06
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 0, 0, 1]),
            ty: TokenType::Literal {
                literal_value: 0x07
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 0, 1, 0]),
            ty: TokenType::Literal {
                literal_value: 0x08
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 0, 1, 1]),
            ty: TokenType::Literal {
                literal_value: 0x09
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 1, 0, 0]),
            ty: TokenType::Literal {
                literal_value: 0x0a
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 1, 0, 1]),
            ty: TokenType::Literal {
                literal_value: 0x0b
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 1, 1, 0]),
            ty: TokenType::Literal {
                literal_value: 0x3a
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 0, 1, 1, 1]),
            ty: TokenType::Literal {
                literal_value: 0x3b
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 0, 0, 0]),
            ty: TokenType::Literal {
                literal_value: 0x3c
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 0, 0, 1]),
            ty: TokenType::Literal {
                literal_value: 0x3d
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 0, 1, 0]),
            ty: TokenType::Literal {
                literal_value: 0x3e
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 0, 1, 1]),
            ty: TokenType::Literal {
                literal_value: 0x3f
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 1, 0, 0]),
            ty: TokenType::Literal {
                literal_value: 0x40
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 1, 0, 1]),
            ty: TokenType::Literal {
                literal_value: 0x80
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 1, 1, 0, 0]),
            ty: TokenType::Literal {
                literal_value: 0x0c
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 1, 1, 0, 1]),
            ty: TokenType::Literal {
                literal_value: 0x38
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 1, 1, 1, 0]),
            ty: TokenType::Literal {
                literal_value: 0x39
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 1, 1, 1, 1, 1, 1, 1]),
            ty: TokenType::Literal {
                literal_value: 0x66
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 0, 0, 1]),
            ty: TokenType::Match {
                distance_value_size: 5,
                distance_base: 0
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 0, 1, 0]),
            ty: TokenType::Match {
                distance_value_size: 7,
                distance_base: 32
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 0, 1, 1]),
            ty: TokenType::Match {
                distance_value_size: 9,
                distance_base: 160
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 0, 0]),
            ty: TokenType::Match {
                distance_value_size: 10,
                distance_base: 672,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 0, 1]),
            ty: TokenType::Match {
                distance_value_size: 12,
                distance_base: 1_696,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 0, 0]),
            ty: TokenType::Match {
                distance_value_size: 14,
                distance_base: 5_792,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 0, 1]),
            ty: TokenType::Match {
                distance_value_size: 15,
                distance_base: 22_176,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 1, 0, 0]),
            ty: TokenType::Match {
                distance_value_size: 18,
                distance_base: 54_944,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 1, 0, 1]),
            ty: TokenType::Match {
                distance_value_size: 20,
                distance_base: 317_088,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 1, 1, 0, 0]),
            ty: TokenType::Match {
                distance_value_size: 20,
                distance_base: 1_365_664,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 1, 1, 0, 1]),
            ty: TokenType::Match {
                distance_value_size: 21,
                distance_base: 2_414_240,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 1, 1, 1, 0, 0]),
            ty: TokenType::Match {
                distance_value_size: 22,
                distance_base: 4_511_392,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 1, 1, 1, 0, 1]),
            ty: TokenType::Match {
                distance_value_size: 23,
                distance_base: 8_705_696,
            },
        },
        Token {
            prefix: slice_to_bitvec(&[1, 0, 1, 1, 1, 1, 1, 1, 0]),
            ty: TokenType::Match {
                distance_value_size: 24,
                distance_base: 17_094_304,
            },
        },
    ];
}

#[derive(Debug, Fail)]
pub enum ZgfxError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid compression type")]
    InvalidCompressionType,
    #[fail(display = "Invalid segmented descriptor")]
    InvalidSegmentedDescriptor,
    #[fail(
        display = "Decompressed size of segments ({}) does not equal to uncompressed size ({})",
        decompressed_size, uncompressed_size
    )]
    InvalidDecompressedSize {
        decompressed_size: usize,
        uncompressed_size: usize,
    },
    #[fail(display = "Token bits not found")]
    TokenBitsNotFound,
}

impl_from_error!(io::Error, ZgfxError, ZgfxError::IOError);

fn slice_to_bitvec(s: &[u8]) -> BitVec<BigEndian, u8> {
    use std::iter::FromIterator;

    BitVec::from_iter(s.iter().map(|v| v == &1))
}
