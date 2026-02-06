//! MPPC (Microsoft Point-to-Point Compression) implementation.
//!
//! Supports RDP4 (8K history) and RDP5 (64K history) compression levels.

pub(crate) mod tables;

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use crate::bitstream::BitStreamReader;
use crate::error::BulkError;
use crate::flags;

use self::tables::{
    HISTORY_BUFFER_SIZE_RDP4, HISTORY_BUFFER_SIZE_RDP5, HISTORY_MASK_RDP4, HISTORY_MASK_RDP5,
    MATCH_BUFFER_SIZE,
};

/// MPPC compression/decompression context.
///
/// Holds the sliding-window history buffer and state needed for MPPC
/// compression and decompression. The history buffer is always allocated
/// at 64 KB, but only the first 8 KB is used for RDP4 mode.
///
/// Ported from FreeRDP's `MPPC_CONTEXT` struct.
pub(crate) struct MppcContext {
    /// Whether this context is for compression (`true`) or decompression (`false`).
    compressor: bool,
    /// Compression level: 0 = RDP4 (8K), 1 = RDP5 (64K).
    compression_level: u32,
    /// Effective history buffer size (8192 for RDP4, 65536 for RDP5).
    history_buffer_size: usize,
    /// History wrapping mask (0x1FFF for RDP4, 0xFFFF for RDP5).
    history_mask: usize,
    /// Sliding-window history buffer (always 64 KB).
    pub(crate) history_buffer: Box<[u8; HISTORY_BUFFER_SIZE_RDP5]>,
    /// Current write position in the history buffer (equivalent to FreeRDP's `HistoryPtr`).
    pub(crate) history_ptr: usize,
    /// History offset used by the compressor for tracking buffer position.
    pub(crate) history_offset: usize,
    /// Match buffer for compression hash table lookups.
    pub(crate) match_buffer: Box<[u16; MATCH_BUFFER_SIZE]>,
}

impl MppcContext {
    /// Creates a new MPPC context.
    ///
    /// `compression_level`: 0 for RDP4 (8K history), 1 for RDP5 (64K history).
    /// `compressor`: `true` if used for compression, `false` for decompression.
    pub(crate) fn new(compression_level: u32, compressor: bool) -> Self {
        let (level, buffer_size, mask) = if compression_level < 1 {
            (0u32, HISTORY_BUFFER_SIZE_RDP4, HISTORY_MASK_RDP4)
        } else {
            (1u32, HISTORY_BUFFER_SIZE_RDP5, HISTORY_MASK_RDP5)
        };

        let mut ctx = Self {
            compressor,
            compression_level: level,
            history_buffer_size: buffer_size,
            history_mask: mask,
            history_buffer: Box::new([0u8; HISTORY_BUFFER_SIZE_RDP5]),
            history_ptr: 0,
            history_offset: 0,
            match_buffer: Box::new([0u16; MATCH_BUFFER_SIZE]),
        };
        ctx.reset(false);
        ctx
    }

    /// Resets the MPPC context.
    ///
    /// Zeros the history buffer and match buffer.
    /// If `flush` is `true`, sets `history_offset` to `history_buffer_size + 1`
    /// (indicating a flush occurred). Otherwise sets `history_offset` to 0.
    /// In both cases, `history_ptr` is reset to 0.
    pub(crate) fn reset(&mut self, flush: bool) {
        self.history_buffer.fill(0);
        self.match_buffer.fill(0);

        if flush {
            self.history_offset = self.history_buffer_size + 1;
            self.history_ptr = 0;
        } else {
            self.history_offset = 0;
            self.history_ptr = 0;
        }
    }

    /// Sets the compression level, adjusting buffer size and mask accordingly.
    pub(crate) fn set_compression_level(&mut self, compression_level: u32) {
        if compression_level < 1 {
            self.compression_level = 0;
            self.history_buffer_size = HISTORY_BUFFER_SIZE_RDP4;
            self.history_mask = HISTORY_MASK_RDP4;
        } else {
            self.compression_level = 1;
            self.history_buffer_size = HISTORY_BUFFER_SIZE_RDP5;
            self.history_mask = HISTORY_MASK_RDP5;
        }
    }

    /// Returns the current compression level (0 = RDP4, 1 = RDP5).
    #[inline]
    pub(crate) fn compression_level(&self) -> u32 {
        self.compression_level
    }

    /// Returns the effective history buffer size.
    #[inline]
    pub(crate) fn history_buffer_size(&self) -> usize {
        self.history_buffer_size
    }

    /// Returns the history wrapping mask.
    #[inline]
    pub(crate) fn history_mask(&self) -> usize {
        self.history_mask
    }

    /// Returns whether this context is a compressor.
    #[inline]
    pub(crate) fn is_compressor(&self) -> bool {
        self.compressor
    }

    /// Decompresses MPPC-compressed data.
    ///
    /// Handles `PACKET_FLUSHED` (reset context), `PACKET_AT_FRONT` (reset history
    /// pointer), literal decoding, CopyOffset decoding (different for RDP4 vs RDP5),
    /// LengthOfMatch decoding, and history buffer copy with wrapping.
    ///
    /// Returns a slice of the decompressed data. For compressed packets this is a
    /// slice into the internal history buffer. For uncompressed packets this is
    /// the source data passed through directly.
    ///
    /// Ported from FreeRDP's `mppc_decompress()` in `libfreerdp/codec/mppc.c`.
    #[expect(
        clippy::as_conversions,
        reason = "bit manipulation requires u32-to-u8/usize casts matching FreeRDP's C code"
    )]
    pub(crate) fn decompress<'a>(
        &'a mut self,
        src_data: &'a [u8],
        flags_value: u32,
    ) -> Result<&'a [u8], BulkError> {
        let history_buffer_size = self.history_buffer_size;
        let compression_level = self.compression_level;
        let history_mask = self.history_mask;

        // Handle PACKET_AT_FRONT: reset history pointer to beginning
        if flags_value & flags::PACKET_AT_FRONT != 0 {
            self.history_offset = 0;
            self.history_ptr = 0;
        }

        // Handle PACKET_FLUSHED: reset context entirely
        if flags_value & flags::PACKET_FLUSHED != 0 {
            self.history_offset = 0;
            self.history_ptr = 0;
            self.history_buffer[..history_buffer_size].fill(0);
        }

        // If data is not compressed, return source data directly
        if flags_value & flags::PACKET_COMPRESSED == 0 {
            return Ok(src_data);
        }

        let mut bs = BitStreamReader::new(src_data);
        let history_buffer_end = history_buffer_size - 1;
        let output_start = self.history_ptr;
        let mut history_ptr = self.history_ptr;

        while bs.remaining_bits() >= 8 {
            let accumulator = bs.accumulator();

            // Check history buffer bounds
            if history_ptr > history_buffer_end {
                return Err(BulkError::HistoryBufferOverflow);
            }

            // --- Literal Encoding ---

            if (accumulator & 0x8000_0000) == 0x0000_0000 {
                // Literal < 0x80: bit 0 followed by lower 7 bits
                let literal = ((accumulator & 0x7F00_0000) >> 24) as u8;
                self.history_buffer[history_ptr] = literal;
                history_ptr += 1;
                bs.shift(8);
                continue;
            } else if (accumulator & 0xC000_0000) == 0x8000_0000 {
                // Literal >= 0x80: bits 10 followed by lower 7 bits
                let literal = (((accumulator & 0x3F80_0000) >> 23) as u8).wrapping_add(0x80);
                self.history_buffer[history_ptr] = literal;
                history_ptr += 1;
                bs.shift(9);
                continue;
            }

            // --- CopyOffset Encoding ---

            let copy_offset: usize;

            if compression_level != 0 {
                // RDP5
                if (accumulator & 0xF800_0000) == 0xF800_0000 {
                    // CopyOffset [0, 63]: bits 11111 + 6 bits
                    copy_offset = ((accumulator >> 21) & 0x3F) as usize;
                    bs.shift(11);
                } else if (accumulator & 0xF800_0000) == 0xF000_0000 {
                    // CopyOffset [64, 319]: bits 11110 + 8 bits
                    copy_offset = ((accumulator >> 19) & 0xFF) as usize + 64;
                    bs.shift(13);
                } else if (accumulator & 0xF000_0000) == 0xE000_0000 {
                    // CopyOffset [320, 2367]: bits 1110 + 11 bits
                    copy_offset = ((accumulator >> 17) & 0x7FF) as usize + 320;
                    bs.shift(15);
                } else if (accumulator & 0xE000_0000) == 0xC000_0000 {
                    // CopyOffset [2368, ]: bits 110 + 16 bits
                    copy_offset = ((accumulator >> 13) & 0xFFFF) as usize + 2368;
                    bs.shift(19);
                } else {
                    return Err(BulkError::InvalidCompressedData("invalid RDP5 CopyOffset encoding"));
                }
            } else {
                // RDP4
                if (accumulator & 0xF000_0000) == 0xF000_0000 {
                    // CopyOffset [0, 63]: bits 1111 + 6 bits
                    copy_offset = ((accumulator >> 22) & 0x3F) as usize;
                    bs.shift(10);
                } else if (accumulator & 0xF000_0000) == 0xE000_0000 {
                    // CopyOffset [64, 319]: bits 1110 + 8 bits
                    copy_offset = ((accumulator >> 20) & 0xFF) as usize + 64;
                    bs.shift(12);
                } else if (accumulator & 0xE000_0000) == 0xC000_0000 {
                    // CopyOffset [320, 8191]: bits 110 + 13 bits
                    copy_offset = ((accumulator >> 16) & 0x1FFF) as usize + 320;
                    bs.shift(16);
                } else {
                    return Err(BulkError::InvalidCompressedData("invalid RDP4 CopyOffset encoding"));
                }
            }

            // --- LengthOfMatch Encoding ---
            // Re-read accumulator after shifting for CopyOffset
            let accumulator = bs.accumulator();
            let length_of_match: usize;

            if (accumulator & 0x8000_0000) == 0x0000_0000 {
                // LengthOfMatch [3]: bit 0
                length_of_match = 3;
                bs.shift(1);
            } else if (accumulator & 0xC000_0000) == 0x8000_0000 {
                // LengthOfMatch [4, 7]: bits 10 + 2 bits
                length_of_match = ((accumulator >> 28) & 0x0003) as usize + 4;
                bs.shift(4);
            } else if (accumulator & 0xE000_0000) == 0xC000_0000 {
                // LengthOfMatch [8, 15]: bits 110 + 3 bits
                length_of_match = ((accumulator >> 26) & 0x0007) as usize + 8;
                bs.shift(6);
            } else if (accumulator & 0xF000_0000) == 0xE000_0000 {
                // LengthOfMatch [16, 31]: bits 1110 + 4 bits
                length_of_match = ((accumulator >> 24) & 0x000F) as usize + 16;
                bs.shift(8);
            } else if (accumulator & 0xF800_0000) == 0xF000_0000 {
                // LengthOfMatch [32, 63]: bits 11110 + 5 bits
                length_of_match = ((accumulator >> 22) & 0x001F) as usize + 32;
                bs.shift(10);
            } else if (accumulator & 0xFC00_0000) == 0xF800_0000 {
                // LengthOfMatch [64, 127]: bits 111110 + 6 bits
                length_of_match = ((accumulator >> 20) & 0x003F) as usize + 64;
                bs.shift(12);
            } else if (accumulator & 0xFE00_0000) == 0xFC00_0000 {
                // LengthOfMatch [128, 255]: bits 1111110 + 7 bits
                length_of_match = ((accumulator >> 18) & 0x007F) as usize + 128;
                bs.shift(14);
            } else if (accumulator & 0xFF00_0000) == 0xFE00_0000 {
                // LengthOfMatch [256, 511]: bits 11111110 + 8 bits
                length_of_match = ((accumulator >> 16) & 0x00FF) as usize + 256;
                bs.shift(16);
            } else if (accumulator & 0xFF80_0000) == 0xFF00_0000 {
                // LengthOfMatch [512, 1023]: bits 111111110 + 9 bits
                length_of_match = ((accumulator >> 14) & 0x01FF) as usize + 512;
                bs.shift(18);
            } else if (accumulator & 0xFFC0_0000) == 0xFF80_0000 {
                // LengthOfMatch [1024, 2047]: bits 1111111110 + 10 bits
                length_of_match = ((accumulator >> 12) & 0x03FF) as usize + 1024;
                bs.shift(20);
            } else if (accumulator & 0xFFE0_0000) == 0xFFC0_0000 {
                // LengthOfMatch [2048, 4095]: bits 11111111110 + 11 bits
                length_of_match = ((accumulator >> 10) & 0x07FF) as usize + 2048;
                bs.shift(22);
            } else if (accumulator & 0xFFF0_0000) == 0xFFE0_0000 {
                // LengthOfMatch [4096, 8191]: bits 111111111110 + 12 bits
                length_of_match = ((accumulator >> 8) & 0x0FFF) as usize + 4096;
                bs.shift(24);
            } else if (accumulator & 0xFFF8_0000) == 0xFFF0_0000 && compression_level != 0 {
                // RDP5 only: LengthOfMatch [8192, 16383]: bits 1111111111110 + 13 bits
                length_of_match = ((accumulator >> 6) & 0x1FFF) as usize + 8192;
                bs.shift(26);
            } else if (accumulator & 0xFFFC_0000) == 0xFFF8_0000 && compression_level != 0 {
                // RDP5 only: LengthOfMatch [16384, 32767]: bits 11111111111110 + 14 bits
                length_of_match = ((accumulator >> 4) & 0x3FFF) as usize + 16384;
                bs.shift(28);
            } else if (accumulator & 0xFFFE_0000) == 0xFFFC_0000 && compression_level != 0 {
                // RDP5 only: LengthOfMatch [32768, 65535]: bits 111111111111110 + 15 bits
                length_of_match = ((accumulator >> 2) & 0x7FFF) as usize + 32768;
                bs.shift(30);
            } else {
                return Err(BulkError::InvalidCompressedData(
                    "invalid LengthOfMatch encoding",
                ));
            }

            // Check that the copy won't overflow the history buffer
            if history_ptr + length_of_match - 1 > history_buffer_end {
                return Err(BulkError::HistoryBufferOverflow);
            }

            // Copy from history buffer at (current - copy_offset) with wrapping
            let mut src_index = (history_ptr.wrapping_sub(copy_offset)) & history_mask;
            for _ in 0..length_of_match {
                self.history_buffer[history_ptr] = self.history_buffer[src_index];
                history_ptr += 1;
                src_index = (src_index + 1) & history_mask;
            }
        }

        let output_end = history_ptr;
        self.history_ptr = history_ptr;

        Ok(&self.history_buffer[output_start..output_end])
    }
}

#[cfg(test)]
mod test_data;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompress_uncompressed_passthrough() {
        let mut ctx = MppcContext::new(0, false);
        let data = b"hello world";
        // No PACKET_COMPRESSED flag → should return source data directly
        let result = ctx.decompress(data, flags::PACKET_AT_FRONT).unwrap();
        assert_eq!(result, b"hello world");
    }

    #[test]
    fn test_decompress_flushed_resets_history() {
        let mut ctx = MppcContext::new(0, false);
        // Write something into the history buffer first
        ctx.history_buffer[0] = 0xAA;
        ctx.history_buffer[1] = 0xBB;
        ctx.history_ptr = 100;
        ctx.history_offset = 50;

        let data = b"test";
        // PACKET_FLUSHED | PACKET_AT_FRONT without PACKET_COMPRESSED
        let flags_value = flags::PACKET_FLUSHED | flags::PACKET_AT_FRONT;
        let result = ctx.decompress(data, flags_value).unwrap();
        // Should return source data (not compressed)
        assert_eq!(result, b"test");
        // History should be zeroed and pointers reset
        assert_eq!(ctx.history_ptr, 0);
        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.history_buffer[0], 0);
        assert_eq!(ctx.history_buffer[1], 0);
    }

    #[test]
    fn test_decompress_at_front_resets_pointer() {
        let mut ctx = MppcContext::new(1, false);
        ctx.history_ptr = 500;
        ctx.history_offset = 200;

        let data = b"data";
        let flags_value = flags::PACKET_AT_FRONT;
        let result = ctx.decompress(data, flags_value).unwrap();
        assert_eq!(result, b"data");
        assert_eq!(ctx.history_ptr, 0);
        assert_eq!(ctx.history_offset, 0);
    }

    /// Ported from FreeRDP's `test_MppcDecompressBellsRdp4`.
    ///
    /// Decompresses `TEST_MPPC_BELLS_RDP4` using RDP4 (8K history)
    /// and verifies the output matches "for.whom.the.bell.tolls,.the.bell.tolls.for.thee!".
    #[test]
    fn test_mppc_decompress_bells_rdp4() {
        let mut ctx = MppcContext::new(0, false);
        // Flags: PACKET_AT_FRONT | PACKET_COMPRESSED (RDP4 — compression level 0)
        let flags_value = flags::PACKET_AT_FRONT | flags::PACKET_COMPRESSED;
        let result = ctx
            .decompress(test_data::TEST_MPPC_BELLS_RDP4, flags_value)
            .unwrap();
        assert_eq!(
            result.len(),
            test_data::TEST_MPPC_BELLS.len(),
            "output size mismatch: actual={}, expected={}",
            result.len(),
            test_data::TEST_MPPC_BELLS.len()
        );
        assert_eq!(
            result,
            test_data::TEST_MPPC_BELLS,
            "MppcDecompressBellsRdp4: output mismatch"
        );
    }

    /// Ported from FreeRDP's `test_MppcDecompressBellsRdp5`.
    ///
    /// Decompresses `TEST_MPPC_BELLS_RDP5` using RDP5 (64K history)
    /// and verifies the output matches "for.whom.the.bell.tolls,.the.bell.tolls.for.thee!".
    #[test]
    fn test_mppc_decompress_bells_rdp5() {
        let mut ctx = MppcContext::new(1, false);
        // Flags: PACKET_AT_FRONT | PACKET_COMPRESSED | 1 (RDP5)
        let flags_value = flags::PACKET_AT_FRONT | flags::PACKET_COMPRESSED | 1;
        let result = ctx
            .decompress(test_data::TEST_MPPC_BELLS_RDP5, flags_value)
            .unwrap();
        assert_eq!(
            result.len(),
            test_data::TEST_MPPC_BELLS.len(),
            "output size mismatch: actual={}, expected={}",
            result.len(),
            test_data::TEST_MPPC_BELLS.len()
        );
        assert_eq!(
            result,
            test_data::TEST_MPPC_BELLS,
            "MppcDecompressBellsRdp5: output mismatch"
        );
    }

    /// Ported from FreeRDP's `test_MppcDecompressBufferRdp5`.
    ///
    /// Decompresses a large binary buffer using RDP5 (64K history)
    /// and verifies byte-for-byte match with the expected uncompressed data.
    #[test]
    fn test_mppc_decompress_buffer_rdp5() {
        let mut ctx = MppcContext::new(1, false);
        // Flags: PACKET_AT_FRONT | PACKET_COMPRESSED | 1 (RDP5)
        let flags_value = flags::PACKET_AT_FRONT | flags::PACKET_COMPRESSED | 1;
        let result = ctx
            .decompress(test_data::TEST_RDP5_COMPRESSED_DATA, flags_value)
            .unwrap();
        assert_eq!(
            result.len(),
            test_data::TEST_RDP5_UNCOMPRESSED_DATA.len(),
            "output size mismatch: actual={}, expected={}",
            result.len(),
            test_data::TEST_RDP5_UNCOMPRESSED_DATA.len()
        );
        assert_eq!(
            result,
            test_data::TEST_RDP5_UNCOMPRESSED_DATA,
            "MppcDecompressBufferRdp5: output mismatch"
        );
    }
}
