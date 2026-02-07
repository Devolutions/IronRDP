//! NCRUSH (RDP 6.0) Huffman-based compression implementation.
//!
//! Uses Huffman coding with an LRU offset cache for LZ77-style
//! back-references. Operates on a 64 KB sliding-window history buffer.
//!
//! Ported from FreeRDP's `libfreerdp/codec/ncrush.c`.

#[cfg(test)]
mod test_data;

pub(crate) mod tables;

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use crate::error::BulkError;

/// History buffer size for NCRUSH (64 KB).
pub(crate) const HISTORY_BUFFER_SIZE: usize = 65536;

/// Hash table size (same as history buffer size).
pub(crate) const HASH_TABLE_SIZE: usize = 65536;

/// Match table size (same as history buffer size).
pub(crate) const MATCH_TABLE_SIZE: usize = 65536;

/// Huffman table for CopyOffset decoding (1024 entries).
pub(crate) const HUFF_TABLE_COPY_OFFSET_SIZE: usize = 1024;

/// Huffman table for LengthOfMatch decoding (4096 entries).
pub(crate) const HUFF_TABLE_LOM_SIZE: usize = 4096;

/// Number of offset cache entries (LRU cache of recent offsets).
pub(crate) const OFFSET_CACHE_SIZE: usize = 4;

/// History buffer fence value used for integrity checking.
pub(crate) const HISTORY_BUFFER_FENCE: u32 = 0xABAB_ABAB;

/// NCRUSH (RDP 6.0) compression/decompression context.
///
/// Maintains a 64 KB sliding-window history buffer, hash/match tables for
/// LZ77 matching, an LRU offset cache, and runtime Huffman tables generated
/// from the static lookup tables.
///
/// Ported from FreeRDP's `NCRUSH_CONTEXT` struct.
pub(crate) struct NCrushContext {
    /// Whether this context is for compression (`true`) or decompression (`false`).
    compressor: bool,
    /// Current write position in the history buffer.
    pub(crate) history_offset: usize,
    /// End offset of valid data in the history buffer (HistoryBufferSize − 1).
    pub(crate) history_end_offset: usize,
    /// Total history buffer size (always 65536).
    pub(crate) history_buffer_size: usize,
    /// 64 KB sliding-window history buffer.
    pub(crate) history_buffer: Box<[u8; HISTORY_BUFFER_SIZE]>,
    /// Integrity fence value (always 0xABABABAB).
    pub(crate) history_buffer_fence: u32,
    /// LRU offset cache for the 4 most recent copy offsets.
    pub(crate) offset_cache: [u32; OFFSET_CACHE_SIZE],
    /// Hash table for 2-byte hash lookups during compression (maps hash → position).
    pub(crate) hash_table: Box<[u16; HASH_TABLE_SIZE]>,
    /// Match table for hash-chain traversal during compression.
    pub(crate) match_table: Box<[u16; MATCH_TABLE_SIZE]>,
    /// Runtime Huffman table for CopyOffset index decoding (generated from
    /// `CopyOffsetBitsLUT`).
    pub(crate) huff_table_copy_offset: Box<[u8; HUFF_TABLE_COPY_OFFSET_SIZE]>,
    /// Runtime Huffman table for LengthOfMatch index decoding (generated from
    /// `LOMBitsLUT`).
    pub(crate) huff_table_lom: Box<[u8; HUFF_TABLE_LOM_SIZE]>,
}

/// Helper to allocate a heap-zeroed boxed array.
fn heap_zeroed_array<const N: usize, T: Default + Copy>() -> Box<[T; N]> {
    // Use vec to avoid stack allocation, then convert to boxed array
    let v: Vec<T> = vec![T::default(); N];
    v.into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!())
}

impl NCrushContext {
    /// Creates a new NCRUSH context.
    ///
    /// `compressor`: `true` for compression contexts, `false` for decompression.
    ///
    /// Allocates the history, hash, and match buffers on the heap, generates
    /// the runtime Huffman tables, and calls `reset(false)`.
    ///
    /// Ported from FreeRDP's `ncrush_context_new`.
    pub(crate) fn new(compressor: bool) -> Result<Self, BulkError> {
        let mut ctx = Self {
            compressor,
            history_offset: 0,
            history_end_offset: HISTORY_BUFFER_SIZE - 1,
            history_buffer_size: HISTORY_BUFFER_SIZE,
            history_buffer: heap_zeroed_array::<HISTORY_BUFFER_SIZE, u8>(),
            history_buffer_fence: HISTORY_BUFFER_FENCE,
            offset_cache: [0u32; OFFSET_CACHE_SIZE],
            hash_table: heap_zeroed_array::<HASH_TABLE_SIZE, u16>(),
            match_table: heap_zeroed_array::<MATCH_TABLE_SIZE, u16>(),
            huff_table_copy_offset: heap_zeroed_array::<HUFF_TABLE_COPY_OFFSET_SIZE, u8>(),
            huff_table_lom: heap_zeroed_array::<HUFF_TABLE_LOM_SIZE, u8>(),
        };

        ctx.generate_tables()?;
        ctx.reset(false);

        Ok(ctx)
    }

    /// Generates the runtime Huffman lookup tables for CopyOffset and
    /// LengthOfMatch decoding.
    ///
    /// Populates `huff_table_lom` from `LOMBitsLUT`/`LOMBaseLUT` and
    /// `huff_table_copy_offset` from `CopyOffsetBitsLUT`.
    ///
    /// Ported from FreeRDP's `ncrush_generate_tables`.
    fn generate_tables(&mut self) -> Result<(), BulkError> {
        // --- Generate HuffTableLOM ---
        // For each LOM index i (0..28), fill entries for all values that
        // map to that index (based on LOMBitsLUT).
        let mut cnt: usize = 0;
        for i in 0u8..28 {
            let bits = tables::LOMBitsLUT[i as usize];
            let num_entries = 1usize << bits;
            for _j in 0..num_entries {
                let l = cnt + 2;
                if l < HUFF_TABLE_LOM_SIZE {
                    self.huff_table_lom[l] = i;
                }
                cnt += 1;
            }
        }

        // Verify the generated LOM table: for each k in [2, 4096), ensure
        // the round-trip: LOMBaseLUT[index] + (k-2) & mask == k.
        for k in 2..HUFF_TABLE_LOM_SIZE {
            let i = if (k - 2) < 768 {
                self.huff_table_lom[k] as usize
            } else {
                28usize
            };

            if i >= tables::LOMBitsLUT.len() || i >= tables::LOMBaseLUT.len() {
                return Err(BulkError::InvalidCompressedData(
                    "NCRUSH: generate_tables LOM index out of range",
                ));
            }

            let mask = (1u32 << tables::LOMBitsLUT[i]) - 1;
            let base = tables::LOMBaseLUT[i];
            let reconstructed = (mask & (k as u32 - 2)) + base;
            if reconstructed != k as u32 {
                return Err(BulkError::InvalidCompressedData(
                    "NCRUSH: generate_tables LOM verification failed",
                ));
            }
        }

        // --- Generate HuffTableCopyOffset ---
        // First 16 indices: direct mapping (no shift)
        let mut k: usize = 0;
        for i in 0u8..16 {
            let bits = tables::CopyOffsetBitsLUT[i as usize];
            let num_entries = 1usize << bits;
            for _j in 0..num_entries {
                let l = k + 2;
                if l < HUFF_TABLE_COPY_OFFSET_SIZE {
                    self.huff_table_copy_offset[l] = i;
                }
                k += 1;
            }
        }

        // Indices 16..32: shifted by 7 bits (>> 7)
        k /= 128;
        for i in 16u8..32 {
            let bits = tables::CopyOffsetBitsLUT[i as usize];
            // bits >= 7 for indices 16..32
            let shift = bits.saturating_sub(7);
            let num_entries = 1usize << shift;
            for _j in 0..num_entries {
                let l = k + 2 + 256;
                if l < HUFF_TABLE_COPY_OFFSET_SIZE {
                    self.huff_table_copy_offset[l] = i;
                }
                k += 1;
            }
        }

        if (k + 256) > HUFF_TABLE_COPY_OFFSET_SIZE {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH: generate_tables CopyOffset overflow",
            ));
        }

        Ok(())
    }

    /// Resets the NCRUSH context.
    ///
    /// Zeros the history buffer, offset cache, match table, and hash table.
    /// If `flush` is `true`, sets `history_offset` to `history_buffer_size + 1`
    /// (sentinel value indicating a flush). Otherwise sets `history_offset` to 0.
    ///
    /// Ported from FreeRDP's `ncrush_context_reset`.
    pub(crate) fn reset(&mut self, flush: bool) {
        self.history_buffer.fill(0);
        self.offset_cache.fill(0);
        self.match_table.fill(0);
        self.hash_table.fill(0);

        if flush {
            self.history_offset = self.history_buffer_size + 1;
        } else {
            self.history_offset = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ncrush_context_new_decompressor() {
        let ctx = NCrushContext::new(false).unwrap();
        assert_eq!(ctx.history_buffer_size, HISTORY_BUFFER_SIZE);
        assert_eq!(ctx.history_end_offset, HISTORY_BUFFER_SIZE - 1);
        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.history_buffer_fence, HISTORY_BUFFER_FENCE);
        assert_eq!(ctx.offset_cache, [0u32; 4]);
        assert!(!ctx.compressor);
    }

    #[test]
    fn test_ncrush_context_new_compressor() {
        let ctx = NCrushContext::new(true).unwrap();
        assert_eq!(ctx.history_buffer_size, HISTORY_BUFFER_SIZE);
        assert!(ctx.compressor);
    }

    #[test]
    fn test_ncrush_context_reset_no_flush() {
        let mut ctx = NCrushContext::new(false).unwrap();
        ctx.history_offset = 12345;
        ctx.offset_cache[0] = 42;
        ctx.offset_cache[1] = 99;
        ctx.history_buffer[100] = 0xFF;

        ctx.reset(false);

        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.offset_cache, [0u32; 4]);
        assert_eq!(ctx.history_buffer[100], 0);
    }

    #[test]
    fn test_ncrush_context_reset_flush() {
        let mut ctx = NCrushContext::new(false).unwrap();
        ctx.reset(true);

        assert_eq!(ctx.history_offset, HISTORY_BUFFER_SIZE + 1);
    }

    #[test]
    fn test_ncrush_generate_tables_lom() {
        let ctx = NCrushContext::new(false).unwrap();

        // First entry at index 2 should be 0 (LOM index 0)
        assert_eq!(ctx.huff_table_lom[2], 0);

        // Spot-check: LOMBitsLUT[0..8] are all 0, meaning each index maps to
        // exactly 1 entry. So indices 2..10 should be 0,1,2,...,7.
        for i in 0..8 {
            assert_eq!(ctx.huff_table_lom[2 + i], i as u8);
        }
    }

    #[test]
    fn test_ncrush_generate_tables_copy_offset() {
        let ctx = NCrushContext::new(false).unwrap();

        // First entry at index 2 should be 0
        assert_eq!(ctx.huff_table_copy_offset[2], 0);

        // CopyOffsetBitsLUT[0..4] are all 0, so 1 entry each.
        // Indices 2..6 should be 0,1,2,3.
        for i in 0..4 {
            assert_eq!(ctx.huff_table_copy_offset[2 + i], i as u8);
        }
    }
}
