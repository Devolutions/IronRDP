//! RDP 8.0 bulk decompression (MS-RDPEGFX 3.1.9.1).
//!
//! This is the algorithm FreeRDP calls ZGFX. The graphics pipeline uses it as is
//! ([`Rdp8Decompressor::new`]). The dynamic virtual channel DYNVC_DATA_FIRST_COMPRESSED and
//! DYNVC_DATA_COMPRESSED PDUs use RDP 8.0 Lite ([`Rdp8Decompressor::new_lite`]), which
//! MS-RDPEDYC 2.2.3.3 defines as the same bitstream with an 8,192-byte history, at most
//! 8,192 uncompressed bytes per segment, a single segment per message, and compression
//! type 0x06.
//!
//! Input is an RDP_SEGMENTED_DATA structure (MS-RDPEGFX 2.2.5.1) holding one or more
//! RDP8_BULK_ENCODED_DATA segments (MS-RDPEGFX 2.2.5.3).

use alloc::vec;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;

/// RDP 8.0 sliding window size: the maximum match distance (MS-RDPEGFX 3.1.9.1).
pub const RDP8_HISTORY_SIZE: usize = 2_500_000;

/// RDP 8.0 Lite sliding window size and per-segment limit (MS-RDPEDYC 2.2.3.3).
pub const RDP8_LITE_HISTORY_SIZE: usize = 8_192;

/// Ceiling on the decompressed output of a single segment.
///
/// A conforming compressor never produces more than 65,535 uncompressed bytes per segment
/// (MS-RDPEGFX 3.1.9.1.2). This is an implementation limit with a wide margin over that,
/// meant only to stop a small hostile segment from expanding into an unbounded allocation.
const MAX_DECOMPRESSED_PER_SEGMENT: usize = 64 * 1024 * 1024;

/// Ceiling on the total decompressed output of a multipart message.
///
/// The declared `uncompressedSize` is sender-controlled, so it cannot bound the allocation.
const MAX_DECOMPRESSED_TOTAL: usize = 256 * 1024 * 1024;

const SEGMENTED_SINGLE: u8 = 0xE0;
const SEGMENTED_MULTIPART: u8 = 0xE1;

const PACKET_COMPR_TYPE_RDP8: u8 = 0x04;
const PACKET_COMPR_TYPE_RDP8_LITE: u8 = 0x06;
const PACKET_COMPRESSED: u8 = 0x20;

/// Error returned by [`Rdp8Decompressor::decompress`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rdp8Error {
    /// The input ended before a complete structure could be read.
    UnexpectedEndOfInput,
    /// The RDP_SEGMENTED_DATA descriptor is neither SINGLE nor MULTIPART, or is MULTIPART
    /// in RDP 8.0 Lite, which allows a single segment only.
    InvalidSegmentedDescriptor(u8),
    /// The segment's compression type is neither RDP 8.0 nor RDP 8.0 Lite.
    UnsupportedCompressionType(u8),
    /// A multipart segment claims more bytes than remain in the input.
    SegmentSizeExceedsBuffer {
        /// Size declared by the segment.
        size: usize,
        /// Bytes left in the input.
        remaining: usize,
    },
    /// The trailing unused-bit count exceeds the bits in the segment.
    InvalidTrailingBitCount(usize),
    /// No token prefix matches the next bits.
    TokenNotFound,
    /// A token needs more bits than remain in the segment.
    IncompleteBitStream {
        /// Bits the token needs.
        needed: usize,
        /// Bits left in the segment.
        remaining: usize,
    },
    /// A match reaches further back than the history buffer.
    MatchDistanceOutOfRange(usize),
    /// A match-length prefix is longer than 14 one bits, which MS-RDPEGFX reserves.
    MatchLengthTooLarge,
    /// A segment decompresses past the per-segment limit: 8,192 bytes in RDP 8.0 Lite,
    /// 64 MiB otherwise.
    SegmentTooLarge,
    /// A multipart message decompresses to more than 256 MiB.
    MessageTooLarge,
    /// The multipart segments do not add up to the declared uncompressed size.
    InvalidDecompressedSize {
        /// Bytes the segments produced.
        decompressed: usize,
        /// Size declared in the multipart header.
        declared: usize,
    },
}

impl fmt::Display for Rdp8Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEndOfInput => write!(f, "unexpected end of input"),
            Self::InvalidSegmentedDescriptor(descriptor) => {
                write!(f, "invalid segmented data descriptor {descriptor:#04x}")
            }
            Self::UnsupportedCompressionType(ty) => write!(f, "unsupported compression type {ty:#04x}"),
            Self::SegmentSizeExceedsBuffer { size, remaining } => write!(
                f,
                "segment claims {size} bytes but only {remaining} bytes remain in the buffer"
            ),
            Self::InvalidTrailingBitCount(count) => {
                write!(f, "trailing unused-bit count {count} exceeds the segment")
            }
            Self::TokenNotFound => write!(f, "no token matches the bitstream"),
            Self::IncompleteBitStream { needed, remaining } => write!(
                f,
                "incomplete bitstream: needed {needed} bits, only {remaining} bits remain"
            ),
            Self::MatchDistanceOutOfRange(distance) => {
                write!(f, "match distance {distance} exceeds the history size")
            }
            Self::MatchLengthTooLarge => write!(f, "match length prefix is too long"),
            Self::SegmentTooLarge => write!(f, "segment decompresses past the per-segment limit"),
            Self::MessageTooLarge => write!(f, "message decompresses past the {MAX_DECOMPRESSED_TOTAL} byte ceiling"),
            Self::InvalidDecompressedSize { decompressed, declared } => write!(
                f,
                "segments decompress to {decompressed} bytes but {declared} bytes were declared"
            ),
        }
    }
}

impl core::error::Error for Rdp8Error {}

/// RDP 8.0 bulk decompressor.
///
/// Holds the history shared by every message decoded with it, so a peer's compression
/// context maps to exactly one decompressor.
pub struct Rdp8Decompressor {
    /// Ring buffer of the most recent output. Its length is the maximum match distance.
    history: Vec<u8>,
    position: usize,
    max_segment_size: usize,
    allow_multipart: bool,
}

impl Rdp8Decompressor {
    /// Creates an RDP 8.0 decompressor, as used by the graphics pipeline.
    ///
    /// Its history takes [`RDP8_HISTORY_SIZE`] bytes.
    pub fn new() -> Self {
        Self {
            history: vec![0; RDP8_HISTORY_SIZE],
            position: 0,
            max_segment_size: MAX_DECOMPRESSED_PER_SEGMENT,
            allow_multipart: true,
        }
    }

    /// Creates an RDP 8.0 Lite decompressor, as used by dynamic virtual channels.
    ///
    /// Its history takes [`RDP8_LITE_HISTORY_SIZE`] bytes. Matches reaching further back,
    /// segments decompressing to more than [`RDP8_LITE_HISTORY_SIZE`] bytes and multipart
    /// messages are rejected (MS-RDPEDYC 2.2.3.3).
    pub fn new_lite() -> Self {
        Self {
            history: vec![0; RDP8_LITE_HISTORY_SIZE],
            position: 0,
            max_segment_size: RDP8_LITE_HISTORY_SIZE,
            allow_multipart: false,
        }
    }

    /// Decompresses an RDP_SEGMENTED_DATA structure and appends the result to `output`.
    ///
    /// Returns the number of bytes appended.
    pub fn decompress(&mut self, input: &[u8], output: &mut Vec<u8>) -> Result<usize, Rdp8Error> {
        let (&descriptor, mut rest) = input.split_first().ok_or(Rdp8Error::UnexpectedEndOfInput)?;

        match descriptor {
            SEGMENTED_SINGLE => self.decompress_segment(rest, output),
            SEGMENTED_MULTIPART if self.allow_multipart => {
                let segment_count = usize::from(u16::from_le_bytes(read_array(&mut rest)?));
                let declared = usize::try_from(u32::from_le_bytes(read_array(&mut rest)?))
                    .map_err(|_| Rdp8Error::MessageTooLarge)?;

                let mut written: usize = 0;
                for _ in 0..segment_count {
                    let size = usize::try_from(u32::from_le_bytes(read_array(&mut rest)?))
                        .map_err(|_| Rdp8Error::UnexpectedEndOfInput)?;
                    let (segment, remaining) =
                        rest.split_at_checked(size).ok_or(Rdp8Error::SegmentSizeExceedsBuffer {
                            size,
                            remaining: rest.len(),
                        })?;
                    rest = remaining;

                    // A segment adds at most 64 MiB, and the total is checked after each one.
                    written += self.decompress_segment(segment, output)?;
                    if written > MAX_DECOMPRESSED_TOTAL {
                        return Err(Rdp8Error::MessageTooLarge);
                    }
                    if written > declared {
                        break;
                    }
                }

                if written == declared {
                    Ok(written)
                } else {
                    Err(Rdp8Error::InvalidDecompressedSize {
                        decompressed: written,
                        declared,
                    })
                }
            }
            other => Err(Rdp8Error::InvalidSegmentedDescriptor(other)),
        }
    }

    /// Decodes one RDP8_BULK_ENCODED_DATA segment.
    fn decompress_segment(&mut self, segment: &[u8], output: &mut Vec<u8>) -> Result<usize, Rdp8Error> {
        let (&header, data) = segment.split_first().ok_or(Rdp8Error::UnexpectedEndOfInput)?;

        let compression_type = header & 0x0F;
        if compression_type != PACKET_COMPR_TYPE_RDP8 && compression_type != PACKET_COMPR_TYPE_RDP8_LITE {
            return Err(Rdp8Error::UnsupportedCompressionType(compression_type));
        }

        if header & PACKET_COMPRESSED == 0 {
            if data.len() > self.max_segment_size {
                return Err(Rdp8Error::SegmentTooLarge);
            }
            self.write_history(data);
            output.extend_from_slice(data);
            return Ok(data.len());
        }

        let Some((&trailer, encoded)) = data.split_last() else {
            return Ok(0);
        };

        // MS-RDPEGFX 3.1.9.1.2.4: the low 3 bits of the final byte count the unused bits of
        // the byte before it; the five high-order bits are reserved.
        let unused_bits = usize::from(trailer & 0x07);
        let bit_count = encoded
            .len()
            .checked_mul(8)
            .and_then(|bits| bits.checked_sub(unused_bits))
            .ok_or(Rdp8Error::InvalidTrailingBitCount(unused_bits))?;

        let mut bits = BitReader::new(encoded, bit_count);
        let mut written: usize = 0;

        while bits.remaining() > 0 {
            let token = TOKENS
                .iter()
                .find(|token| bits.peek(token.prefix_len) == Some(u32::from(token.prefix)))
                .ok_or(Rdp8Error::TokenNotFound)?;
            bits.skip(token.prefix_len);

            let remaining_budget = self.max_segment_size - written;
            written += match token.kind {
                TokenKind::NullLiteral => {
                    let value = bits.read(8)?;
                    let value = u8::try_from(value).expect("8 bits fit in a byte");
                    self.push_literal(value, output);
                    1
                }
                TokenKind::Literal(value) => {
                    self.push_literal(value, output);
                    1
                }
                TokenKind::Match {
                    distance_bits,
                    distance_base,
                } => {
                    // At most 2^24 - 1 + 17,094,304, so neither the sum nor the conversion fails.
                    let distance = usize::try_from(bits.read(distance_bits)? + distance_base)
                        .expect("match distance fits in usize");

                    if distance == 0 {
                        self.copy_unencoded(&mut bits, remaining_budget, output)?
                    } else {
                        self.copy_match(&mut bits, distance, remaining_budget, output)?
                    }
                }
            };

            if written > self.max_segment_size {
                return Err(Rdp8Error::SegmentTooLarge);
            }
        }

        Ok(written)
    }

    fn push_literal(&mut self, value: u8, output: &mut Vec<u8>) {
        self.history[self.position] = value;
        self.advance(1);
        output.push(value);
    }

    /// A match distance of zero introduces a run of raw bytes: a 15-bit count, padding to
    /// the next byte boundary, then the bytes themselves.
    fn copy_unencoded(
        &mut self,
        bits: &mut BitReader<'_>,
        remaining_budget: usize,
        output: &mut Vec<u8>,
    ) -> Result<usize, Rdp8Error> {
        let length = usize::try_from(bits.read(15)?).expect("15 bits fit in usize");
        if length > remaining_budget {
            return Err(Rdp8Error::SegmentTooLarge);
        }

        bits.align_to_byte()?;
        let bytes = bits.read_bytes(length)?;
        self.write_history(bytes);
        output.extend_from_slice(bytes);

        Ok(length)
    }

    /// Copies `length` bytes starting `distance` bytes back in the history. The copy may
    /// overlap the bytes it produces, which repeats the last `distance` bytes.
    fn copy_match(
        &mut self,
        bits: &mut BitReader<'_>,
        distance: usize,
        remaining_budget: usize,
        output: &mut Vec<u8>,
    ) -> Result<usize, Rdp8Error> {
        if distance > self.history.len() {
            return Err(Rdp8Error::MatchDistanceOutOfRange(distance));
        }

        // The length is a run of N one bits and a zero bit, then N + 1 value bits:
        // length = 2^(N + 1) + value, or 3 when N is 0. MS-RDPEGFX 3.1.9.1.2.4 defines N up
        // to 14 (lengths up to 65,535) and reserves longer prefixes.
        let prefix = bits.leading_ones();
        if prefix > 14 {
            return Err(Rdp8Error::MatchLengthTooLarge);
        }
        bits.read(prefix + 1)?;
        let length = if prefix == 0 {
            3
        } else {
            let value_bits = prefix + 1;
            let value = usize::try_from(bits.read(value_bits)?).expect("15 bits fit in usize");
            (1usize << value_bits) + value
        };

        if length > remaining_budget {
            return Err(Rdp8Error::SegmentTooLarge);
        }

        output.reserve(length);
        let history_size = self.history.len();
        let mut source = (self.position + history_size - distance) % history_size;
        for _ in 0..length {
            let value = self.history[source];
            self.history[self.position] = value;
            self.advance(1);
            output.push(value);

            source += 1;
            if source == history_size {
                source = 0;
            }
        }

        Ok(length)
    }

    fn write_history(&mut self, mut data: &[u8]) {
        let history_size = self.history.len();
        if data.len() > history_size {
            let skipped = data.len() - history_size;
            self.advance(skipped % history_size);
            data = &data[skipped..];
        }

        let to_end = history_size - self.position;
        if data.len() <= to_end {
            self.history[self.position..self.position + data.len()].copy_from_slice(data);
        } else {
            let (back, front) = data.split_at(to_end);
            self.history[self.position..].copy_from_slice(back);
            self.history[..front.len()].copy_from_slice(front);
        }
        self.advance(data.len());
    }

    fn advance(&mut self, count: usize) {
        self.position = (self.position + count) % self.history.len();
    }
}

impl Default for Rdp8Decompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Rdp8Decompressor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rdp8Decompressor")
            .field("history_size", &self.history.len())
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

fn read_array<const N: usize>(input: &mut &[u8]) -> Result<[u8; N], Rdp8Error> {
    let (bytes, rest) = input.split_first_chunk::<N>().ok_or(Rdp8Error::UnexpectedEndOfInput)?;
    *input = rest;
    Ok(*bytes)
}

/// Reads bits most-significant first, up to `end` bits into `data`.
///
/// Separate from [`crate::bitstream::BitStreamReader`], which serves the MPPC, NCRUSH and
/// XCRUSH hot paths: RDP 8.0 also needs a hard end-of-bits limit (the trailer byte),
/// counting leading one bits, byte alignment and raw byte runs.
struct BitReader<'a> {
    data: &'a [u8],
    position: usize,
    end: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], end: usize) -> Self {
        Self { data, position: 0, end }
    }

    fn remaining(&self) -> usize {
        self.end - self.position
    }

    /// Returns the next `count` bits (at most 32) without consuming them, or `None` if
    /// fewer remain.
    fn peek(&self, count: usize) -> Option<u32> {
        debug_assert!(count <= 32);
        if count > self.remaining() {
            return None;
        }
        if count == 0 {
            return Some(0);
        }

        // Five bytes cover any 32-bit read starting at an arbitrary bit offset.
        let first = self.position / 8;
        let window = (0..5).fold(0u64, |window, i| {
            (window << 8) | u64::from(self.data.get(first + i).copied().unwrap_or(0))
        });
        let shift = 40 - self.position % 8 - count;
        let value = (window >> shift) & ((1u64 << count) - 1);

        Some(u32::try_from(value).expect("at most 32 bits"))
    }

    fn skip(&mut self, count: usize) {
        debug_assert!(count <= self.remaining());
        self.position += count;
    }

    fn read(&mut self, count: usize) -> Result<u32, Rdp8Error> {
        let value = self.peek(count).ok_or_else(|| Rdp8Error::IncompleteBitStream {
            needed: count,
            remaining: self.remaining(),
        })?;
        self.skip(count);
        Ok(value)
    }

    fn leading_ones(&self) -> usize {
        (self.position..self.end)
            .take_while(|&bit| self.data[bit / 8] & (0x80 >> (bit % 8)) != 0)
            .count()
    }

    fn align_to_byte(&mut self) -> Result<(), Rdp8Error> {
        let padding = (8 - self.position % 8) % 8;
        self.read(padding).map(|_| ())
    }

    /// Reads `count` whole bytes. The reader must be byte-aligned.
    fn read_bytes(&mut self, count: usize) -> Result<&'a [u8], Rdp8Error> {
        debug_assert_eq!(self.position % 8, 0);
        let needed = count.checked_mul(8).ok_or(Rdp8Error::UnexpectedEndOfInput)?;
        if needed > self.remaining() {
            return Err(Rdp8Error::IncompleteBitStream {
                needed,
                remaining: self.remaining(),
            });
        }

        let start = self.position / 8;
        self.position += needed;
        Ok(&self.data[start..start + count])
    }
}

#[derive(Clone, Copy)]
enum TokenKind {
    /// A "0" prefix followed by the 8-bit literal value.
    NullLiteral,
    Literal(u8),
    Match {
        distance_bits: usize,
        distance_base: u32,
    },
}

struct Token {
    prefix: u16,
    prefix_len: usize,
    kind: TokenKind,
}

const fn literal(prefix: u16, prefix_len: usize, value: u8) -> Token {
    Token {
        prefix,
        prefix_len,
        kind: TokenKind::Literal(value),
    }
}

const fn distance(prefix: u16, prefix_len: usize, distance_bits: usize, distance_base: u32) -> Token {
    Token {
        prefix,
        prefix_len,
        kind: TokenKind::Match {
            distance_bits,
            distance_base,
        },
    }
}

/// MS-RDPEGFX 3.1.9.1.2.1 token table. The prefixes form a prefix-free code.
const TOKENS: [Token; 40] = [
    Token {
        prefix: 0b0,
        prefix_len: 1,
        kind: TokenKind::NullLiteral,
    },
    literal(0b11000, 5, 0x00),
    literal(0b11001, 5, 0x01),
    literal(0b110100, 6, 0x02),
    literal(0b110101, 6, 0x03),
    literal(0b110110, 6, 0xFF),
    literal(0b1101110, 7, 0x04),
    literal(0b1101111, 7, 0x05),
    literal(0b1110000, 7, 0x06),
    literal(0b1110001, 7, 0x07),
    literal(0b1110010, 7, 0x08),
    literal(0b1110011, 7, 0x09),
    literal(0b1110100, 7, 0x0A),
    literal(0b1110101, 7, 0x0B),
    literal(0b1110110, 7, 0x3A),
    literal(0b1110111, 7, 0x3B),
    literal(0b1111000, 7, 0x3C),
    literal(0b1111001, 7, 0x3D),
    literal(0b1111010, 7, 0x3E),
    literal(0b1111011, 7, 0x3F),
    literal(0b1111100, 7, 0x40),
    literal(0b1111101, 7, 0x80),
    literal(0b11111100, 8, 0x0C),
    literal(0b11111101, 8, 0x38),
    literal(0b11111110, 8, 0x39),
    literal(0b11111111, 8, 0x66),
    distance(0b10001, 5, 5, 0),
    distance(0b10010, 5, 7, 32),
    distance(0b10011, 5, 9, 160),
    distance(0b10100, 5, 10, 672),
    distance(0b10101, 5, 12, 1_696),
    distance(0b101100, 6, 14, 5_792),
    distance(0b101101, 6, 15, 22_176),
    distance(0b1011100, 7, 18, 54_944),
    distance(0b1011101, 7, 20, 317_088),
    distance(0b10111100, 8, 20, 1_365_664),
    distance(0b10111101, 8, 21, 2_414_240),
    distance(0b101111100, 9, 22, 4_511_392),
    distance(0b101111101, 9, 23, 8_705_696),
    distance(0b101111110, 9, 24, 17_094_304),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Wraps an encoded bitstream as a single compressed RDP 8.0 segment.
    fn single_compressed(compression_type: u8, bitstream: &[u8]) -> Vec<u8> {
        let mut input = vec![SEGMENTED_SINGLE, PACKET_COMPRESSED | compression_type];
        input.extend_from_slice(bitstream);
        input
    }

    /// Packs a string of '0' and '1' (spaces ignored) into a bitstream with its trailer byte.
    fn pack(bits: &str) -> Vec<u8> {
        let bits: Vec<bool> = bits.chars().filter(|c| *c != ' ').map(|c| c == '1').collect();
        let mut bytes: Vec<u8> = bits
            .chunks(8)
            .map(|chunk| {
                chunk
                    .iter()
                    .enumerate()
                    .fold(0u8, |byte, (i, &bit)| byte | (u8::from(bit) << (7 - i)))
            })
            .collect();
        bytes.push(u8::try_from((8 - bits.len() % 8) % 8).unwrap());
        bytes
    }

    fn decompress(input: &[u8]) -> Result<Vec<u8>, Rdp8Error> {
        let mut output = Vec::new();
        let written = Rdp8Decompressor::new().decompress(input, &mut output)?;
        assert_eq!(written, output.len());
        Ok(output)
    }

    #[test]
    fn decodes_short_literal() {
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8, &[0b1100_1000, 0x03]);
        assert_eq!(decompress(&input).unwrap(), [0x01]);
    }

    #[test]
    fn ignores_reserved_trailer_bits() {
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8, &[0b1100_1000, 0x03 | 0b1010_0000]);
        assert_eq!(decompress(&input).unwrap(), [0x01]);
    }

    #[test]
    fn decodes_null_prefixed_literal() {
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8, &[0b0011_0010, 0b1000_0000, 0x07]);
        assert_eq!(decompress(&input).unwrap(), [0x65]);
    }

    #[test]
    fn decodes_multiple_literals() {
        let input = single_compressed(
            PACKET_COMPR_TYPE_RDP8,
            &[0b1100_1110, 0b1001_1011, 0b0001_1001, 0b0100_0000, 0x06],
        );
        assert_eq!(decompress(&input).unwrap(), [0x01, 0x02, 0xFF, 0x65]);
    }

    #[test]
    fn decodes_overlapping_match() {
        // Literal 0x65, then a match at distance 1 with length 4.
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8, &[0b0011_0010, 0b1100_0100, 0b0011_0000, 0x01]);
        assert_eq!(decompress(&input).unwrap(), [0x65; 5]);
    }

    #[test]
    fn decodes_long_match() {
        // "ABC", then a match at distance 3 with length 57.
        let input = single_compressed(
            PACKET_COMPR_TYPE_RDP8,
            &[
                0b0010_0000,
                0b1001_0000,
                0b1000_1000,
                0b0111_0001,
                0b0001_1111,
                0b1011_0010,
                0x01,
            ],
        );
        assert_eq!(decompress(&input).unwrap(), "ABC".repeat(20).as_bytes());
    }

    #[test]
    fn decodes_unencoded_run() {
        let expected = b"The quick brown fox jumps over the lazy dog";
        let mut bitstream = vec![0b1000_1000, 0b0000_0000, 0b0001_0101, 0b1000_0000];
        bitstream.extend_from_slice(expected);
        bitstream.push(0x00);

        let input = single_compressed(PACKET_COMPR_TYPE_RDP8, &bitstream);
        assert_eq!(decompress(&input).unwrap(), expected);
    }

    #[test]
    fn accepts_rdp8_lite() {
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8_LITE, &[0b1100_1000, 0x03]);
        assert_eq!(decompress(&input).unwrap(), [0x01]);
    }

    #[test]
    fn rejects_other_compression_types() {
        let input = single_compressed(0x01, &[0b1100_1000, 0x03]);
        assert_eq!(decompress(&input), Err(Rdp8Error::UnsupportedCompressionType(0x01)));
    }

    #[test]
    fn copies_uncompressed_segment_into_history() {
        let mut decompressor = Rdp8Decompressor::new();
        let mut output = Vec::new();

        let raw = [SEGMENTED_SINGLE, PACKET_COMPR_TYPE_RDP8_LITE, b'x', b'y', b'z'];
        decompressor.decompress(&raw, &mut output).unwrap();
        assert_eq!(output, b"xyz");

        // A match at distance 3 with length 3 refers to the previous message.
        output.clear();
        let matched = single_compressed(PACKET_COMPR_TYPE_RDP8_LITE, &[0b1000_1000, 0b1100_0000, 0x05]);
        decompressor.decompress(&matched, &mut output).unwrap();
        assert_eq!(output, b"xyz");
    }

    /// MS-RDPEDYC 4.3.3 and 4.3.4: two RDP 8.0 Lite segments, the second matching against
    /// the history left by the first.
    #[test]
    fn decodes_rdpedyc_samples_with_shared_history() {
        let mut decompressor = Rdp8Decompressor::new_lite();
        let mut output = Vec::new();

        let first = [0xE0, 0x26, 0x38, 0xC4, 0x3F, 0xF4, 0x74, 0x01];
        assert_eq!(decompressor.decompress(&first, &mut output), Ok(1595));
        assert!(output.iter().all(|&b| b == b'q'));

        output.clear();
        let second = [0xE0, 0x26, 0x88, 0x7F, 0xE8, 0xF4, 0x02];
        assert_eq!(decompressor.decompress(&second, &mut output), Ok(1597));
        assert!(output.iter().all(|&b| b == b'q'));
    }

    // Match token 101100 has a 14-bit distance based at 5,792; a trailing 0 bit means length 3.
    const MATCH_DISTANCE_8192: &str = "101100 00100101100000 0";
    const MATCH_DISTANCE_8193: &str = "101100 00100101100001 0";

    #[test]
    fn lite_limits_match_distance_to_its_history() {
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8_LITE, &pack(MATCH_DISTANCE_8193));
        assert_eq!(
            Rdp8Decompressor::new_lite().decompress(&input, &mut Vec::new()),
            Err(Rdp8Error::MatchDistanceOutOfRange(8193))
        );
        // The same match is within the RDP 8.0 history.
        assert_eq!(decompress(&input).unwrap(), [0; 3]);
    }

    #[test]
    fn lite_history_holds_the_last_8192_bytes() {
        let mut decompressor = Rdp8Decompressor::new_lite();
        let data: Vec<u8> = (0..15_000u32).map(|i| u8::try_from(i % 251).unwrap()).collect();
        for chunk in data.chunks(5_000) {
            let mut raw = vec![SEGMENTED_SINGLE, PACKET_COMPR_TYPE_RDP8_LITE];
            raw.extend_from_slice(chunk);
            decompressor.decompress(&raw, &mut Vec::new()).unwrap();
        }

        let mut output = Vec::new();
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8_LITE, &pack(MATCH_DISTANCE_8192));
        decompressor.decompress(&input, &mut output).unwrap();
        assert_eq!(output, data[data.len() - 8192..][..3]);
    }

    #[test]
    fn lite_limits_segment_size() {
        let mut raw = vec![SEGMENTED_SINGLE, PACKET_COMPR_TYPE_RDP8_LITE];
        raw.extend_from_slice(&[0x55; 8192]);
        assert_eq!(Rdp8Decompressor::new_lite().decompress(&raw, &mut Vec::new()), Ok(8192));
        raw.push(0x55);
        assert_eq!(
            Rdp8Decompressor::new_lite().decompress(&raw, &mut Vec::new()),
            Err(Rdp8Error::SegmentTooLarge)
        );

        // Literal "q", then a match at distance 1 with length 2^13: 8,193 bytes in all.
        let expanding = pack("0 01110001  10001 00001  111111111111 0 0000000000000");
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8_LITE, &expanding);
        assert_eq!(
            Rdp8Decompressor::new_lite().decompress(&input, &mut Vec::new()),
            Err(Rdp8Error::SegmentTooLarge)
        );
        assert_eq!(decompress(&input).unwrap().len(), 8193);
    }

    #[test]
    fn rejects_reserved_match_length_prefixes() {
        // Literal "q", then a match at distance 1 whose length prefix has N one bits.
        let with_prefix = |n: usize| {
            let bits = format!("0 01110001 10001 00001 {}0 {}", "1".repeat(n), "0".repeat(n + 1));
            single_compressed(PACKET_COMPR_TYPE_RDP8, &pack(&bits))
        };

        // N = 14 is the longest defined prefix: 2^15 bytes plus the literal.
        assert_eq!(decompress(&with_prefix(14)).unwrap().len(), 32_769);
        assert_eq!(decompress(&with_prefix(15)), Err(Rdp8Error::MatchLengthTooLarge));
    }

    #[test]
    fn lite_rejects_multipart() {
        let mut input = vec![SEGMENTED_MULTIPART, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00];
        input.extend_from_slice(&3u32.to_le_bytes());
        input.extend_from_slice(&[PACKET_COMPR_TYPE_RDP8_LITE, b'a', b'b']);
        assert_eq!(
            Rdp8Decompressor::new_lite().decompress(&input, &mut Vec::new()),
            Err(Rdp8Error::InvalidSegmentedDescriptor(SEGMENTED_MULTIPART))
        );
    }

    #[test]
    fn decodes_multipart() {
        let mut input = vec![SEGMENTED_MULTIPART, 0x02, 0x00, 0x04, 0x00, 0x00, 0x00];
        for part in [b"ab", b"cd"] {
            input.extend_from_slice(&3u32.to_le_bytes());
            input.push(PACKET_COMPR_TYPE_RDP8);
            input.extend_from_slice(part);
        }
        assert_eq!(decompress(&input).unwrap(), b"abcd");
    }

    #[test]
    fn rejects_multipart_size_mismatch() {
        let mut input = vec![SEGMENTED_MULTIPART, 0x01, 0x00, 0x05, 0x00, 0x00, 0x00];
        input.extend_from_slice(&3u32.to_le_bytes());
        input.extend_from_slice(&[PACKET_COMPR_TYPE_RDP8, b'a', b'b']);
        assert_eq!(
            decompress(&input),
            Err(Rdp8Error::InvalidDecompressedSize {
                decompressed: 2,
                declared: 5
            })
        );
    }

    #[test]
    fn rejects_truncated_input() {
        assert_eq!(decompress(&[]), Err(Rdp8Error::UnexpectedEndOfInput));
        assert_eq!(decompress(&[SEGMENTED_SINGLE]), Err(Rdp8Error::UnexpectedEndOfInput));
        assert_eq!(decompress(&[0x12]), Err(Rdp8Error::InvalidSegmentedDescriptor(0x12)));

        // A null literal cut off after 4 of its 8 value bits.
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8, &[0b0001_0000, 0x03]);
        assert!(matches!(
            decompress(&input),
            Err(Rdp8Error::IncompleteBitStream { needed: 8, .. })
        ));
    }

    #[test]
    fn rejects_trailer_larger_than_bitstream() {
        let input = single_compressed(PACKET_COMPR_TYPE_RDP8, &[0x07]);
        assert_eq!(decompress(&input), Err(Rdp8Error::InvalidTrailingBitCount(7)));
    }
}
