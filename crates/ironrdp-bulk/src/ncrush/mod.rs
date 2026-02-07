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

    /// Refills the bit accumulator from the source data.
    ///
    /// NCRUSH uses **LSB-first (little-endian)** bit packing — bits are consumed
    /// from the least-significant end of the `bits` accumulator. When `nbits`
    /// drops below 16, this function reads 1 or 2 bytes from `src[src_pos..]`
    /// and appends them to the high end of the accumulator.
    ///
    /// Returns `false` if the source is exhausted AND `nbits` is negative
    /// (irrecoverable underflow). Returns `true` otherwise.
    ///
    /// Ported from FreeRDP's `NCrushFetchBits`.
    fn fetch_bits(src: &[u8], src_pos: &mut usize, nbits: &mut i32, bits: &mut u32) -> bool {
        if *nbits < 16 {
            let remaining = src.len().saturating_sub(*src_pos);
            match remaining {
                0 => {
                    // No more source bytes — only fail if we've consumed
                    // more bits than were available (negative nbits).
                    return *nbits >= 0;
                }
                1 => {
                    // Single byte available
                    let byte_val = src[*src_pos] as u32;
                    *src_pos += 1;
                    if *nbits >= 0 {
                        *bits = bits.wrapping_add(byte_val << (*nbits as u32));
                    }
                    *nbits += 8;
                }
                _ => {
                    // Two or more bytes available — read a 16-bit word (LE)
                    let lo = src[*src_pos] as u32;
                    *src_pos += 1;
                    let hi = src[*src_pos] as u32;
                    *src_pos += 1;
                    let word = lo | (hi << 8);
                    *bits = bits.wrapping_add(word << (*nbits as u32));
                    *nbits += 16;
                }
            }
        }
        true
    }

    /// Decompresses an NCRUSH-compressed packet.
    ///
    /// `src_data` contains the raw packet data (possibly compressed).
    /// `flags_value` contains control flags (`PACKET_COMPRESSED`,
    /// `PACKET_FLUSHED`, `PACKET_AT_FRONT`).
    ///
    /// Returns a slice of the decompressed data. For non-compressed packets,
    /// returns a slice of the input. For compressed packets, returns a slice
    /// into the internal history buffer.
    ///
    /// Ported from FreeRDP's `ncrush_decompress`.
    pub(crate) fn decompress<'a>(
        &'a mut self,
        src_data: &'a [u8],
        flags_value: u32,
    ) -> Result<&'a [u8], BulkError> {
        use crate::flags;

        if self.history_end_offset != HISTORY_BUFFER_SIZE - 1 {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH: invalid history end offset",
            ));
        }

        let history_end = self.history_end_offset; // 65535

        // Handle PACKET_AT_FRONT: slide window — move last 32 KB to the front
        if flags_value & flags::PACKET_AT_FRONT != 0 {
            if self.history_offset <= 32768 {
                return Err(BulkError::InvalidCompressedData(
                    "NCRUSH: history offset too small for AT_FRONT",
                ));
            }
            let src_start = self.history_offset - 32768;
            self.history_buffer
                .copy_within(src_start..src_start + 32768, 0);
            self.history_offset = 32768;
            self.history_buffer[32768..HISTORY_BUFFER_SIZE].fill(0);
        }

        // Handle PACKET_FLUSHED: reset history and offset cache
        if flags_value & flags::PACKET_FLUSHED != 0 {
            self.history_offset = 0;
            self.history_buffer.fill(0);
            self.offset_cache.fill(0);
        }

        // If not compressed, return source data directly
        if flags_value & flags::PACKET_COMPRESSED == 0 {
            return Ok(src_data);
        }

        if src_data.len() < 4 {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH: compressed input too short (< 4 bytes)",
            ));
        }

        let history_start = self.history_offset;
        let mut history_ptr = self.history_offset;

        // --- Bit accumulator initialisation (first 4 bytes, little-endian) ---
        let mut bits =
            u32::from_le_bytes([src_data[0], src_data[1], src_data[2], src_data[3]]);
        let mut nbits: i32 = 32;
        let mut src_pos: usize = 4;

        // Masks for Huffman table lookups (from HuffTableMask)
        const LEC_MASK: u32 = 0x1FFF; // 13-bit mask for HuffTableLEC[8192]
        const LOM_MASK: u32 = 0x01FF; //  9-bit mask for HuffTableLOM[512]

        let mut index_lec: u32;

        // ===== Main decompression loop =====
        loop {
            // --- Inner loop: decode literals until a non-literal symbol ---
            loop {
                let masked_bits = (bits & LEC_MASK) as usize;
                if masked_bits >= tables::HuffTableLEC.len() {
                    return Err(BulkError::InvalidCompressedData(
                        "NCRUSH: LEC masked bits out of range",
                    ));
                }

                let lec_entry = tables::HuffTableLEC[masked_bits];
                index_lec = (lec_entry & 0xFFF) as u32;
                let bit_length = (lec_entry >> 12) as u32;
                bits >>= bit_length;
                nbits -= bit_length as i32;

                if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                    return Err(BulkError::UnexpectedEndOfInput);
                }

                if index_lec >= 256 {
                    break;
                }

                // Literal byte
                if history_ptr >= history_end {
                    return Err(BulkError::HistoryBufferOverflow);
                }

                self.history_buffer[history_ptr] = (lec_entry & 0xFF) as u8;
                history_ptr += 1;
            }

            // End-of-stream marker (symbol 256)
            if index_lec == 256 {
                break;
            }

            // --- Decode CopyOffset and LengthOfMatch ---
            let copy_offset_index = index_lec - 257;

            let copy_offset: u32;
            let length_of_match_base: u32;

            if copy_offset_index >= 32 {
                // --- Offset Cache Hit (LEC symbols 289–292) ---
                let cache_index = (index_lec - 289) as usize;
                if cache_index >= OFFSET_CACHE_SIZE {
                    return Err(BulkError::InvalidCompressedData(
                        "NCRUSH: offset cache index out of range",
                    ));
                }

                copy_offset = self.offset_cache[cache_index];

                // Decode LengthOfMatch from HuffTableLOM
                let lom_masked = (bits & LOM_MASK) as usize;
                if lom_masked >= tables::HuffTableLOM.len() {
                    return Err(BulkError::InvalidCompressedData(
                        "NCRUSH: LOM index out of range (cache path)",
                    ));
                }
                let lom_entry = tables::HuffTableLOM[lom_masked];
                let length_of_match_idx = (lom_entry & 0xFFF) as usize;
                let bit_length = (lom_entry >> 12) as u32;
                bits >>= bit_length;
                nbits -= bit_length as i32;

                if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                    return Err(BulkError::UnexpectedEndOfInput);
                }

                if length_of_match_idx >= tables::LOMBitsLUT.len()
                    || length_of_match_idx >= tables::LOMBaseLUT.len()
                {
                    return Err(BulkError::InvalidCompressedData(
                        "NCRUSH: LOM lookup out of range",
                    ));
                }

                let lom_bits = tables::LOMBitsLUT[length_of_match_idx];
                let mut lom_base = tables::LOMBaseLUT[length_of_match_idx];

                if lom_bits > 0 {
                    let extra_mask = (1u32 << lom_bits) - 1;
                    lom_base += bits & extra_mask;
                    bits >>= lom_bits;
                    nbits -= lom_bits as i32;

                    if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                        return Err(BulkError::UnexpectedEndOfInput);
                    }
                }

                length_of_match_base = lom_base;

                // LRU cache update: swap cache_index entry to the front
                let old = self.offset_cache[cache_index];
                self.offset_cache[cache_index] = self.offset_cache[0];
                self.offset_cache[0] = old;
            } else {
                // --- Regular CopyOffset (LEC symbols 257–288) ---
                let coi = copy_offset_index as usize;
                if coi >= tables::CopyOffsetBitsLUT.len()
                    || coi >= tables::CopyOffsetBaseLUT.len()
                {
                    return Err(BulkError::InvalidCompressedData(
                        "NCRUSH: CopyOffset lookup out of range",
                    ));
                }

                let co_bits = tables::CopyOffsetBitsLUT[coi];
                let co_base = tables::CopyOffsetBaseLUT[coi];

                copy_offset = if co_bits > 0 {
                    let extra_mask = (1u32 << co_bits) - 1;
                    let extra = bits & extra_mask;
                    let tmp = co_base + extra;
                    if tmp < 1 {
                        return Err(BulkError::InvalidCompressedData(
                            "NCRUSH: CopyOffset underflow",
                        ));
                    }
                    bits >>= co_bits;
                    nbits -= co_bits as i32;

                    if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                        return Err(BulkError::UnexpectedEndOfInput);
                    }

                    tmp - 1
                } else {
                    co_base - 1
                };

                // Decode LengthOfMatch from HuffTableLOM
                let lom_masked = (bits & LOM_MASK) as usize;
                if lom_masked >= tables::HuffTableLOM.len() {
                    return Err(BulkError::InvalidCompressedData(
                        "NCRUSH: LOM index out of range (offset path)",
                    ));
                }
                let lom_entry = tables::HuffTableLOM[lom_masked];
                let length_of_match_idx = (lom_entry & 0xFFF) as usize;
                let bit_length = (lom_entry >> 12) as u32;
                bits >>= bit_length;
                nbits -= bit_length as i32;

                if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                    return Err(BulkError::UnexpectedEndOfInput);
                }

                if length_of_match_idx >= tables::LOMBitsLUT.len()
                    || length_of_match_idx >= tables::LOMBaseLUT.len()
                {
                    return Err(BulkError::InvalidCompressedData(
                        "NCRUSH: LOM lookup out of range",
                    ));
                }

                let lom_bits = tables::LOMBitsLUT[length_of_match_idx];
                let mut lom_base = tables::LOMBaseLUT[length_of_match_idx];

                if lom_bits > 0 {
                    let extra_mask = (1u32 << lom_bits) - 1;
                    lom_base += bits & extra_mask;
                    bits >>= lom_bits;
                    nbits -= lom_bits as i32;

                    if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                        return Err(BulkError::UnexpectedEndOfInput);
                    }
                }

                length_of_match_base = lom_base;

                // Push new offset into cache (shift down, insert at front)
                self.offset_cache[3] = self.offset_cache[2];
                self.offset_cache[2] = self.offset_cache[1];
                self.offset_cache[1] = self.offset_cache[0];
                self.offset_cache[0] = copy_offset;
            }

            // --- Perform history buffer copy ---
            let length_of_match = length_of_match_base as usize;
            let copy_offset_usize = copy_offset as usize;

            if length_of_match < 2 {
                return Err(BulkError::InvalidCompressedData(
                    "NCRUSH: match length < 2",
                ));
            }

            // Bounds check (ported from FreeRDP's -1006 error).
            // The wrapped source address and the destination must both have
            // enough room for the full match length within the buffer.
            let copy_src_wrapped = history_ptr.wrapping_sub(copy_offset_usize) & 0xFFFF;
            if length_of_match > history_end
                || copy_src_wrapped >= (history_end - length_of_match)
                || history_ptr >= (history_end - length_of_match)
            {
                return Err(BulkError::HistoryBufferOverflow);
            }

            let copy_length = core::cmp::min(length_of_match, copy_offset_usize);

            if history_ptr >= copy_offset_usize {
                // --- No-wrap case: source is within the current buffer ---
                let src_start = history_ptr - copy_offset_usize;

                // Copy first min(length, offset) bytes.
                // Byte-by-byte for correct LZ77 overlap semantics.
                for i in 0..copy_length {
                    self.history_buffer[history_ptr] = self.history_buffer[src_start + i];
                    history_ptr += 1;
                }

                // Handle repeating pattern (LZ77 overlap: length > offset).
                // After the first copy, the freshly written bytes at
                // [match_start .. match_start + offset] form the pattern
                // that repeats cyclically.
                if length_of_match > copy_offset_usize {
                    let pattern_start = src_start + copy_offset_usize; // = original history_ptr
                    let mut idx = 0usize;
                    let mut remaining = length_of_match;
                    while remaining > copy_offset_usize {
                        if idx >= copy_offset_usize {
                            idx = 0;
                        }
                        self.history_buffer[history_ptr] =
                            self.history_buffer[pattern_start + idx];
                        history_ptr += 1;
                        idx += 1;
                        remaining -= 1;
                    }
                }
            } else {
                // --- Wrap case: source wraps around the buffer boundary ---
                // This path is reached when CopyOffset > history_ptr,
                // meaning the reference reaches back past the start of
                // the current write position (into data from a previous
                // packet, placed by PACKET_AT_FRONT).
                let wrap_src =
                    history_end - (copy_offset_usize - history_ptr) + 1;

                let mut src_idx = wrap_src;
                let mut cl = copy_length;

                // Copy from end of buffer until buffer end or copy_length
                while cl > 0 && src_idx <= history_end {
                    self.history_buffer[history_ptr] = self.history_buffer[src_idx];
                    history_ptr += 1;
                    src_idx += 1;
                    cl -= 1;
                }

                // If copy_length wasn't exhausted (source wrapped around
                // to the beginning), continue from position 0.
                // NOTE: per FreeRDP, this continuation is folded into the
                // repeat loop below. The bounds check guarantees this
                // path is not reached when LengthOfMatch <= CopyOffset.
                src_idx = 0;
                while cl > 0 {
                    self.history_buffer[history_ptr] = self.history_buffer[src_idx];
                    history_ptr += 1;
                    src_idx += 1;
                    cl -= 1;
                }

                // Handle repeating pattern from beginning of buffer
                if length_of_match > copy_offset_usize {
                    let mut idx = 0usize;
                    let mut remaining = length_of_match;
                    while remaining > copy_offset_usize {
                        if idx >= copy_offset_usize {
                            idx = 0;
                        }
                        self.history_buffer[history_ptr] = self.history_buffer[idx];
                        history_ptr += 1;
                        idx += 1;
                        remaining -= 1;
                    }
                }
            }
        }

        // Verify end-of-stream marker
        if index_lec != 256 {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH: stream did not end with EOS marker",
            ));
        }

        // Verify history buffer fence (detects buffer overflows)
        if self.history_buffer_fence != HISTORY_BUFFER_FENCE {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH: history buffer fence overwritten",
            ));
        }

        self.history_offset = history_ptr;
        Ok(&self.history_buffer[history_start..history_ptr])
    }

    /// Adds source data positions to the hash table and match table.
    ///
    /// For each position in `[history_offset, history_offset + src_size - 8)`:
    /// - Computes a 2-byte hash from the source data (little-endian u16).
    /// - Stores the old hash table entry into `match_table[position]`
    ///   (creating a chain of positions with the same hash).
    /// - Updates `hash_table[hash]` with the new position.
    ///
    /// Ported from FreeRDP's `ncrush_hash_table_add`.
    pub(crate) fn hash_table_add(
        &mut self,
        src_data: &[u8],
        src_size: usize,
        history_offset: usize,
    ) {
        if src_size < 8 {
            return;
        }
        let end_offset = history_offset + src_size - 8;
        let mut offset = history_offset;
        let mut src_idx = 0usize;

        while offset < end_offset {
            let hash =
                u16::from_le_bytes([src_data[src_idx], src_data[src_idx + 1]]) as usize;
            let old_entry = self.hash_table[hash];
            self.hash_table[hash] = offset as u16;
            self.match_table[offset] = old_entry;
            src_idx += 1;
            offset += 1;
        }
    }

    /// Computes the match length between two positions in the history buffer.
    ///
    /// Compares bytes starting at `offset1` and `offset2`, stopping when a
    /// mismatch is found or `offset1` exceeds `limit`. Returns the number
    /// of matching bytes (may be negative if the limit is exceeded
    /// immediately, indicating no valid comparison was possible).
    ///
    /// Ported from FreeRDP's `ncrush_find_match_length`.
    fn find_match_length(&self, offset1: usize, offset2: usize, limit: usize) -> i32 {
        let buf = &*self.history_buffer;
        let start = offset1;
        let mut i1 = offset1;
        let mut i2 = offset2;

        loop {
            if i1 > limit {
                break;
            }
            let v1 = buf[i1];
            let v2 = buf[i2];
            i1 += 1;
            i2 += 1;
            if v1 != v2 {
                break;
            }
        }

        // Equivalent to FreeRDP's `Ptr1 - (Ptr + 1)`
        (i1 as i32) - (start as i32) - 1
    }

    /// Finds the best LZ77 match for the current position using hash-chain
    /// traversal.
    ///
    /// Searches up to 4 candidates from the hash chain, using a quick filter
    /// (checking the byte at the current best match length) before computing
    /// full match lengths. Returns `None` if no match is found, or
    /// `Some((match_length, match_offset))` for the best match.
    ///
    /// A match length > 16 is considered "good enough" and terminates the
    /// search early.
    ///
    /// Ported from FreeRDP's `ncrush_find_best_match`.
    pub(crate) fn find_best_match(
        &mut self,
        history_offset: u16,
    ) -> Result<Option<(usize, u16)>, BulkError> {
        let ho = history_offset as usize;

        if self.match_table[ho] == 0 {
            return Ok(None);
        }

        let mut match_length: usize = 2;
        let mut offset: u16 = history_offset;
        let history_ptr = self.history_offset; // end of valid data

        // Sentinel: allows the chain-following logic to work at position 0
        self.match_table[0] = history_offset;
        let mut match_offset: u16 = self.match_table[ho];
        let mut next_offset: u16 = self.match_table[offset as usize];

        for _i in 0..4 {
            let mut j: i32 = -1;

            // 6 chain-following steps with quick-filter check.
            // Each step follows the chain one link and checks if the
            // candidate's byte at position `match_length` matches the
            // current position's byte at `history_offset + match_length`.
            // Alternates between Offset and NextOffset.
            let target_byte = self.history_buffer[ho + match_length];

            if j < 0 {
                offset = self.match_table[next_offset as usize];
                if self.history_buffer[match_length + next_offset as usize] == target_byte {
                    j = 0;
                }
            }
            if j < 0 {
                next_offset = self.match_table[offset as usize];
                if self.history_buffer[match_length + offset as usize] == target_byte {
                    j = 1;
                }
            }
            if j < 0 {
                offset = self.match_table[next_offset as usize];
                if self.history_buffer[match_length + next_offset as usize] == target_byte {
                    j = 2;
                }
            }
            if j < 0 {
                next_offset = self.match_table[offset as usize];
                if self.history_buffer[match_length + offset as usize] == target_byte {
                    j = 3;
                }
            }
            if j < 0 {
                offset = self.match_table[next_offset as usize];
                if self.history_buffer[match_length + next_offset as usize] == target_byte {
                    j = 4;
                }
            }
            if j < 0 {
                next_offset = self.match_table[offset as usize];
                if self.history_buffer[match_length + offset as usize] == target_byte {
                    j = 5;
                }
            }

            if j >= 0 {
                // Pick the candidate: even j → NextOffset, odd j → Offset
                if (j % 2) == 0 {
                    offset = next_offset;
                }

                if (offset != history_offset) && (offset != 0) {
                    let len = self.find_match_length(
                        ho + 2,
                        offset as usize + 2,
                        history_ptr,
                    );
                    let length = (len + 2) as usize;

                    if (len + 2) < 2 {
                        // Boundary error — clean up and return error
                        self.match_table[0] = 0;
                        return Err(BulkError::InvalidCompressedData(
                            "NCRUSH: match length computation error",
                        ));
                    }

                    if length > 16 {
                        // Great match — update and stop
                        match_length = length;
                        match_offset = offset;
                        break;
                    }

                    if length > match_length {
                        match_length = length;
                        match_offset = offset;
                    }

                    if (length <= match_length) || (ho + 2 < history_ptr) {
                        next_offset = self.match_table[offset as usize];
                        // match_length may have changed; next iteration
                        // will recompute target_byte
                        continue;
                    }
                }

                break;
            }
            // j < 0: no candidate passed the quick filter in this batch
            // of 6 chain steps. Continue to next outer iteration (the
            // chain pointers have already advanced).
        }

        self.match_table[0] = 0; // Clean up sentinel
        Ok(Some((match_length, match_offset)))
    }

    /// Slides the encoder window by moving the last 32 KB of history to the
    /// front, and adjusting all hash/match table entries accordingly.
    ///
    /// Called when the history buffer is nearly full to make room for new data
    /// while preserving the most recent 32 KB for back-references.
    ///
    /// Ported from FreeRDP's `ncrush_move_encoder_windows`.
    pub(crate) fn move_encoder_windows(
        &mut self,
        history_ptr: usize,
    ) -> Result<(), BulkError> {
        const HALF: usize = HISTORY_BUFFER_SIZE / 2; // 32768

        if history_ptr < HALF || history_ptr > HISTORY_BUFFER_SIZE {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH: invalid history ptr for window move",
            ));
        }

        // Move last 32 KB to front
        self.history_buffer
            .copy_within((history_ptr - HALF)..history_ptr, 0);

        let history_offset = (history_ptr - HALF) as i32;

        // Adjust hash table entries: subtract the offset shift
        for entry in self.hash_table.iter_mut() {
            let new_val = (*entry as i32) - history_offset;
            *entry = if new_val <= 0 { 0 } else { new_val as u16 };
        }

        // Adjust match table entries (relocate first half)
        const MATCH_HALF: usize = MATCH_TABLE_SIZE / 2;
        for j in 0..MATCH_HALF {
            let src_idx = (history_offset as usize) + j;
            if src_idx >= MATCH_TABLE_SIZE {
                continue;
            }
            let new_val = (self.match_table[src_idx] as i32) - history_offset;
            self.match_table[j] = if new_val <= 0 { 0 } else { new_val as u16 };
        }

        // Zero upper half of match table
        self.match_table[MATCH_HALF..MATCH_TABLE_SIZE].fill(0);

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

    // --- decompress tests ---

    #[test]
    fn test_ncrush_decompress_uncompressed_passthrough() {
        use crate::flags;

        let mut ctx = NCrushContext::new(false).unwrap();
        let data = b"hello world";

        // No PACKET_COMPRESSED flag → should return source data directly
        let result = ctx
            .decompress(data, flags::PACKET_FLUSHED)
            .unwrap();
        assert_eq!(result, b"hello world");
        // History offset should remain 0 (no decompression occurred)
        assert_eq!(ctx.history_offset, 0);
    }

    #[test]
    fn test_ncrush_decompress_flushed_clears_state() {
        use crate::flags;

        let mut ctx = NCrushContext::new(false).unwrap();
        ctx.history_offset = 1000;
        ctx.offset_cache[0] = 42;
        ctx.history_buffer[500] = 0xFF;

        let data = b"test";
        let _result = ctx
            .decompress(data, flags::PACKET_FLUSHED)
            .unwrap();

        // PACKET_FLUSHED should clear history and offset cache
        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.offset_cache, [0u32; 4]);
        assert_eq!(ctx.history_buffer[500], 0);
    }

    #[test]
    fn test_ncrush_decompress_compressed_too_short() {
        use crate::flags;

        let mut ctx = NCrushContext::new(false).unwrap();
        let data = [0u8; 3]; // less than 4 bytes

        let result = ctx.decompress(&data, flags::PACKET_FLUSHED | flags::PACKET_COMPRESSED);
        assert!(result.is_err());
    }

    #[test]
    fn test_ncrush_decompress_fetch_bits_basic() {
        // Test the fetch_bits helper directly
        let src = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let mut src_pos = 0usize;
        let mut nbits: i32 = 8;
        let mut bits: u32 = 0x12;

        // nbits >= 16, no fetch needed
        let mut nbits2: i32 = 20;
        let mut bits2: u32 = 0x12345;
        let mut src_pos2 = 0usize;
        assert!(NCrushContext::fetch_bits(
            &src, &mut src_pos2, &mut nbits2, &mut bits2
        ));
        assert_eq!(nbits2, 20); // unchanged
        assert_eq!(bits2, 0x12345); // unchanged
        assert_eq!(src_pos2, 0); // no bytes consumed

        // nbits < 16, fetch 2 bytes
        assert!(NCrushContext::fetch_bits(
            &src, &mut src_pos, &mut nbits, &mut bits
        ));
        assert_eq!(nbits, 24); // 8 + 16
        assert_eq!(src_pos, 2);
        // bits = 0x12 + (0xAA | (0xBB << 8)) << 8
        //      = 0x12 + 0xBBAA << 8
        //      = 0x12 + 0xBBAA00
        //      = 0xBBAA12
        assert_eq!(bits, 0x00BBAA12);
    }

    #[test]
    fn test_ncrush_decompress_fetch_bits_single_byte() {
        let src = [0x42];
        let mut src_pos = 0usize;
        let mut nbits: i32 = 5;
        let mut bits: u32 = 0x1F;

        assert!(NCrushContext::fetch_bits(
            &src, &mut src_pos, &mut nbits, &mut bits
        ));
        assert_eq!(nbits, 13); // 5 + 8
        assert_eq!(src_pos, 1);
        // bits = 0x1F + (0x42 << 5)
        //      = 0x1F + 0x840
        //      = 0x85F
        assert_eq!(bits, 0x85F);
    }

    #[test]
    fn test_ncrush_decompress_fetch_bits_exhausted_ok() {
        let src: [u8; 0] = [];
        let mut src_pos = 0usize;
        let mut nbits: i32 = 5;
        let mut bits: u32 = 0x1F;

        // No more data but nbits >= 0 → ok
        assert!(NCrushContext::fetch_bits(
            &src, &mut src_pos, &mut nbits, &mut bits
        ));
        assert_eq!(nbits, 5); // unchanged
    }

    #[test]
    fn test_ncrush_decompress_fetch_bits_exhausted_fail() {
        let src: [u8; 0] = [];
        let mut src_pos = 0usize;
        let mut nbits: i32 = -1;
        let mut bits: u32 = 0;

        // No more data and nbits < 0 → fail
        assert!(!NCrushContext::fetch_bits(
            &src, &mut src_pos, &mut nbits, &mut bits
        ));
    }

    /// Byte-exact decompression test ported from FreeRDP
    /// `test_NCrushDecompressBells` in `TestFreeRDPCodecNCrush.c`.
    ///
    /// Verifies that NCRUSH decompression of the compressed "bells" data
    /// produces the original plaintext byte-for-byte.
    #[test]
    fn test_ncrush_decompress_bells() {
        use crate::flags;

        let mut ctx = NCrushContext::new(false).unwrap();

        // FreeRDP flags: PACKET_COMPRESSED | 2 (compression type NCRUSH)
        let flags_value = flags::PACKET_COMPRESSED | 0x02;

        let result = ctx
            .decompress(test_data::TEST_BELLS_NCRUSH, flags_value)
            .unwrap();

        assert_eq!(
            result.len(),
            test_data::TEST_BELLS_DATA.len(),
            "output size mismatch: got {}, expected {}",
            result.len(),
            test_data::TEST_BELLS_DATA.len()
        );

        assert_eq!(
            result,
            test_data::TEST_BELLS_DATA,
            "NCrushDecompressBells: output mismatch"
        );
    }

    // --- Match-finding tests ---

    #[test]
    fn test_ncrush_hash_table_add_basic() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // Write "ABABAB..." into history at offset 100
        let data = b"ABABABABAB"; // 10 bytes
        ctx.hash_table_add(data, data.len(), 100);

        // The 2-byte hash for "AB" is u16::from_le_bytes([0x41, 0x42]) = 0x4241
        let hash_ab = u16::from_le_bytes([b'A', b'B']) as usize;

        // The last occurrence of "AB" should be at the highest offset
        // that was inserted. With src_size=10, end_offset = 100+10-8 = 102.
        // So we insert at offsets 100, 101.
        // "AB" appears at offset 100 (data[0..2]) only; offset 101 would
        // hash "BA" which is different.
        let hash_ba = u16::from_le_bytes([b'B', b'A']) as usize;

        // hash_table[hash_ab] should point to offset 100
        // (only "AB" at position 100 — the later "AB" at 102 is not inserted
        //  because end_offset = 102 and the while condition is offset < end_offset)
        // Actually let's trace: offset starts at 100, end = 102
        //   offset=100: hash("AB")=0x4241, insert 100
        //   offset=101: hash("BA")=0x4142, insert 101
        //   offset=102: 102 >= 102, stop
        // Wait, the condition is offset < end_offset, so:
        //   100 < 102 → yes, process
        //   101 < 102 → yes, process
        //   102 < 102 → no, stop
        // So only 2 positions are inserted.

        // For hash "AB" (0x4241): hash_table[0x4241] = 100
        assert_eq!(ctx.hash_table[hash_ab], 100);
        // For hash "BA" (0x4142): hash_table[0x4142] = 101
        assert_eq!(ctx.hash_table[hash_ba], 101);
    }

    #[test]
    fn test_ncrush_hash_table_add_chain() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // Insert two blocks with the same starting bytes to create a chain
        let data1 = b"XYXYXYXYXY"; // 10 bytes at offset 50
        ctx.hash_table_add(data1, data1.len(), 50);

        let data2 = b"XYXYXYXYXY"; // 10 bytes at offset 200
        ctx.hash_table_add(data2, data2.len(), 200);

        let hash_xy = u16::from_le_bytes([b'X', b'Y']) as usize;

        // hash_table[hash_xy] should point to most recent (200)
        assert_eq!(ctx.hash_table[hash_xy], 200);

        // match_table[200] should chain back to 50
        assert_eq!(ctx.match_table[200], 50);
    }

    #[test]
    fn test_ncrush_find_match_length_basic() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // Write identical data at two positions
        ctx.history_buffer[10] = b'A';
        ctx.history_buffer[11] = b'B';
        ctx.history_buffer[12] = b'C';
        ctx.history_buffer[13] = b'D';
        ctx.history_buffer[14] = b'X'; // mismatch

        ctx.history_buffer[20] = b'A';
        ctx.history_buffer[21] = b'B';
        ctx.history_buffer[22] = b'C';
        ctx.history_buffer[23] = b'D';
        ctx.history_buffer[24] = b'Y'; // mismatch

        ctx.history_offset = 30; // limit

        // Match from offset 10 and 20: 4 bytes match (A, B, C, D), then mismatch
        let len = ctx.find_match_length(10, 20, 30);
        assert_eq!(len, 4);
    }

    #[test]
    fn test_ncrush_find_match_length_limit() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // Write identical data at two positions
        for i in 0..10 {
            ctx.history_buffer[100 + i] = (i as u8) + 1;
            ctx.history_buffer[200 + i] = (i as u8) + 1;
        }

        // With limit = 104, we can compare indices 100..104 (5 checks).
        // All 5 bytes match, but then 105 > 104, so we break.
        // Return: (105 - 100) - 1 = 4
        let len = ctx.find_match_length(100, 200, 104);
        assert_eq!(len, 4);
    }

    #[test]
    fn test_ncrush_find_match_length_immediate_limit() {
        let ctx = NCrushContext::new(true).unwrap();

        // offset1 > limit immediately → returns -1
        let len = ctx.find_match_length(10, 20, 5);
        assert_eq!(len, -1);
    }

    #[test]
    fn test_ncrush_find_best_match_no_chain() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // match_table[100] = 0 → no chain
        let result = ctx.find_best_match(100).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_ncrush_find_best_match_simple() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // Set up: write "ABCDEF" at position 50 and "ABCDXY" at position 100
        let pattern1 = b"ABCDEF";
        let pattern2 = b"ABCDXY";
        for (i, &b) in pattern1.iter().enumerate() {
            ctx.history_buffer[50 + i] = b;
        }
        for (i, &b) in pattern2.iter().enumerate() {
            ctx.history_buffer[100 + i] = b;
        }

        // Set history_offset (write cursor) past the data
        ctx.history_offset = 110;

        // Create a hash chain: match_table[100] = 50 (position 100 chains to 50)
        ctx.match_table[100] = 50;

        // The first 2 bytes match the hash; find_best_match starts comparing
        // from offset+2. Bytes at 52,53 match 102,103 (C,D), then mismatch (E vs X).
        // So match length = 4 (A,B,C,D).
        let result = ctx.find_best_match(100).unwrap();
        assert!(result.is_some());
        let (length, offset) = result.unwrap();
        assert_eq!(length, 4);
        assert_eq!(offset, 50);
    }

    #[test]
    fn test_ncrush_move_encoder_windows_basic() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // Write some data in the second half of the buffer
        for i in 0..100 {
            ctx.history_buffer[32768 + i] = (i as u8) + 1;
        }

        // Set up hash and match table entries pointing into second half
        ctx.hash_table[0x1234] = 32800; // points to position 32800
        ctx.match_table[32800] = 32790; // chains to position 32790

        // Slide window: history_ptr = 32868 (100 bytes past the half point)
        ctx.move_encoder_windows(32868).unwrap();

        // Data should now be at the front: positions 32768..32868 → 0..100
        // But actually, copy_within copies (32868 - 32768)..32868 = 100..32868
        // Wait, let me recalculate.
        // HALF = 32768, history_ptr = 32868
        // Source: (32868 - 32768)..32868 = 100..32868
        // Dest: 0..

        // Actually, the function copies history_buffer[(history_ptr - HALF)..history_ptr]
        // = history_buffer[100..32868] to position 0.
        // history_offset = history_ptr - HALF = 100
        // Hash table entries are adjusted: 32800 - 100 = 32700
        assert_eq!(ctx.hash_table[0x1234], 32700);
    }

    #[test]
    fn test_ncrush_move_encoder_windows_clamps_negative() {
        let mut ctx = NCrushContext::new(true).unwrap();

        // Entry pointing before the offset should be clamped to 0
        ctx.hash_table[42] = 50; // 50 < offset (say, 100)

        ctx.move_encoder_windows(32868).unwrap();
        // history_offset = 32868 - 32768 = 100
        // 50 - 100 = -50 → clamped to 0
        assert_eq!(ctx.hash_table[42], 0);
    }
}
