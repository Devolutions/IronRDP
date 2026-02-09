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
use alloc::{boxed::Box, vec, vec::Vec};

use crate::error::BulkError;

/// LSB-first (little-endian) bit writer for NCRUSH compression.
///
/// Bits are accumulated from the least-significant side. When the
/// accumulator reaches ≥ 16 bits, the lower 16 bits are flushed as
/// two little-endian bytes to the output buffer.
///
/// Ported from FreeRDP's `NCrushWriteStart`/`NCrushWriteBits`/`NCrushWriteFinish`.
pub(crate) struct NCrushBitWriter<'a> {
    /// Output byte buffer.
    dst: &'a mut [u8],
    /// Current write position in `dst`.
    pos: usize,
    /// Bit accumulator (bits are packed from LSB upward).
    accumulator: u32,
    /// Number of valid bits in the accumulator.
    offset: u32,
}

impl<'a> NCrushBitWriter<'a> {
    /// Creates a new bit writer targeting the given output buffer.
    pub(crate) fn new(dst: &'a mut [u8]) -> Self {
        Self {
            dst,
            pos: 0,
            accumulator: 0,
            offset: 0,
        }
    }

    /// Writes `nbits` bits from the low end of `bits` into the stream.
    ///
    /// When the accumulator reaches ≥ 16 bits, the lower 16 bits are
    /// flushed as 2 bytes (little-endian) to the output buffer.
    ///
    /// Returns `Err` if the output buffer overflows.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "intentional truncation: lower 16 bits of u32 accumulator flushed as u16 LE"
    )]
    pub(crate) fn write_bits(&mut self, bits: u32, nbits: u32) -> Result<(), BulkError> {
        self.accumulator |= bits << self.offset;
        self.offset += nbits;

        if self.offset >= 16 {
            if self.pos + 2 > self.dst.len() {
                return Err(BulkError::OutputBufferTooSmall {
                    required: self.pos + 2,
                    available: self.dst.len(),
                });
            }
            let le_bytes = (self.accumulator as u16).to_le_bytes();
            self.dst[self.pos] = le_bytes[0];
            self.dst[self.pos + 1] = le_bytes[1];
            self.pos += 2;
            self.accumulator >>= 16;
            self.offset -= 16;
        }

        Ok(())
    }

    /// Flushes any remaining bits in the accumulator to the output buffer.
    ///
    /// Always writes 2 bytes (the lower 16 bits of the accumulator).
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "intentional truncation: lower 16 bits of u32 accumulator flushed as u16 LE"
    )]
    pub(crate) fn finish(&mut self) -> Result<(), BulkError> {
        if self.pos + 2 > self.dst.len() {
            return Err(BulkError::OutputBufferTooSmall {
                required: self.pos + 2,
                available: self.dst.len(),
            });
        }
        let le_bytes = (self.accumulator as u16).to_le_bytes();
        self.dst[self.pos] = le_bytes[0];
        self.dst[self.pos + 1] = le_bytes[1];
        self.pos += 2;
        Ok(())
    }

    /// Returns the number of bytes written so far (including any `finish` call).
    pub(crate) fn bytes_written(&self) -> usize {
        self.pos
    }

    /// Returns `true` if writing `n` more bytes would overflow the buffer.
    pub(crate) fn would_overflow(&self, n: usize) -> bool {
        self.pos + n > self.dst.len()
    }
}

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
#[expect(
    clippy::unnecessary_box_returns,
    reason = "Box return is intentional: arrays up to 64 KB must be heap-allocated to avoid stack overflow"
)]
fn heap_zeroed_array<const N: usize, T: Default + Copy>() -> Box<[T; N]> {
    // Use vec to avoid stack allocation, then convert to boxed array
    let v: Vec<T> = vec![T::default(); N];
    v.into_boxed_slice().try_into().unwrap_or_else(|_| unreachable!())
}

impl NCrushContext {
    /// Creates a new NCRUSH context.
    ///
    /// Allocates the history, hash, and match buffers on the heap, generates
    /// the runtime Huffman tables, and calls `reset(false)`.
    ///
    /// Ported from FreeRDP's `ncrush_context_new`.
    pub(crate) fn new() -> Result<Self, BulkError> {
        let mut ctx = Self {
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
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "table generation: k (usize ≤4096) safely cast to u32 for verification"
    )]
    fn generate_tables(&mut self) -> Result<(), BulkError> {
        // --- Generate HuffTableLOM ---
        // For each LOM index i (0..28), fill entries for all values that
        // map to that index (based on LOMBitsLUT).
        let mut cnt: usize = 0;
        for i in 0u8..28 {
            let bits = tables::LOMBitsLUT[usize::from(i)];
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
                usize::from(self.huff_table_lom[k])
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
            let bits = tables::CopyOffsetBitsLUT[usize::from(i)];
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
            let bits = tables::CopyOffsetBitsLUT[usize::from(i)];
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
    #[expect(
        clippy::as_conversions,
        clippy::cast_sign_loss,
        reason = "*nbits (i32) cast to u32 for shift; always non-negative when used"
    )]
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
                    let byte_val = u32::from(src[*src_pos]);
                    *src_pos += 1;
                    if *nbits >= 0 {
                        *bits = bits.wrapping_add(byte_val << (*nbits as u32));
                    }
                    *nbits += 8;
                }
                _ => {
                    // Two or more bytes available — read a 16-bit word (LE)
                    let lo = u32::from(src[*src_pos]);
                    *src_pos += 1;
                    let hi = u32::from(src[*src_pos]);
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
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "Huffman decode: masked u32 values safely narrowed to u8/usize; \
                  bit_length/lom_bits (u32, ≤15) safely cast to i32; \
                  copy_offset/length_of_match (u32 ≤65535) safely widen to usize"
    )]
    pub(crate) fn decompress<'a>(&'a mut self, src_data: &'a [u8], flags_value: u32) -> Result<&'a [u8], BulkError> {
        use crate::flags;

        if self.history_end_offset != HISTORY_BUFFER_SIZE - 1 {
            return Err(BulkError::InvalidCompressedData("NCRUSH: invalid history end offset"));
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
            self.history_buffer.copy_within(src_start..src_start + 32768, 0);
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
        let mut bits = u32::from_le_bytes([src_data[0], src_data[1], src_data[2], src_data[3]]);
        let mut nbits: i32 = 32;
        let mut src_pos: usize = 4;

        // Masks for Huffman table lookups
        const LEC_MASK: u32 = 0x1FFF; // 13-bit mask for HuffTableLEC[8192]
        const LOM_MASK: u32 = 0x01FF; //  9-bit mask for HuffTableLOM[512]

        let mut index_lec: u32;

        // ===== Main decompression loop =====
        loop {
            // --- Inner loop: decode literals until a non-literal symbol ---
            loop {
                let masked_bits = (bits & LEC_MASK) as usize;
                if masked_bits >= tables::HuffTableLEC.len() {
                    return Err(BulkError::InvalidCompressedData("NCRUSH: LEC masked bits out of range"));
                }

                let lec_entry = tables::HuffTableLEC[masked_bits];
                index_lec = u32::from(lec_entry & 0xFFF);
                let bit_length = u32::from(lec_entry >> 12);
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

                self.history_buffer[history_ptr] = lec_entry as u8; // lower 8 bits of u16
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
                let length_of_match_idx = usize::from(lom_entry & 0xFFF);
                let bit_length = u32::from(lom_entry >> 12);
                bits >>= bit_length;
                nbits -= bit_length as i32;

                if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                    return Err(BulkError::UnexpectedEndOfInput);
                }

                if length_of_match_idx >= tables::LOMBitsLUT.len() || length_of_match_idx >= tables::LOMBaseLUT.len() {
                    return Err(BulkError::InvalidCompressedData("NCRUSH: LOM lookup out of range"));
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
                self.offset_cache.swap(cache_index, 0);
            } else {
                // --- Regular CopyOffset (LEC symbols 257–288) ---
                let coi = copy_offset_index as usize;
                if coi >= tables::CopyOffsetBitsLUT.len() || coi >= tables::CopyOffsetBaseLUT.len() {
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
                        return Err(BulkError::InvalidCompressedData("NCRUSH: CopyOffset underflow"));
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
                let length_of_match_idx = usize::from(lom_entry & 0xFFF);
                let bit_length = u32::from(lom_entry >> 12);
                bits >>= bit_length;
                nbits -= bit_length as i32;

                if !Self::fetch_bits(src_data, &mut src_pos, &mut nbits, &mut bits) {
                    return Err(BulkError::UnexpectedEndOfInput);
                }

                if length_of_match_idx >= tables::LOMBitsLUT.len() || length_of_match_idx >= tables::LOMBaseLUT.len() {
                    return Err(BulkError::InvalidCompressedData("NCRUSH: LOM lookup out of range"));
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
                return Err(BulkError::InvalidCompressedData("NCRUSH: match length < 2"));
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

                if length_of_match <= copy_offset_usize {
                    // Fast path: no overlap — bulk copy.
                    self.history_buffer
                        .copy_within(src_start..src_start + copy_length, history_ptr);
                    history_ptr += copy_length;
                } else {
                    // Slow path: LZ77 overlap (length > offset).
                    // Must copy byte-by-byte: earlier output feeds later input.
                    for i in 0..copy_length {
                        self.history_buffer[history_ptr] = self.history_buffer[src_start + i];
                        history_ptr += 1;
                    }

                    // Handle repeating pattern (overlap).
                    let pattern_start = src_start + copy_offset_usize;
                    let mut idx = 0usize;
                    let mut remaining = length_of_match;
                    while remaining > copy_offset_usize {
                        if idx >= copy_offset_usize {
                            idx = 0;
                        }
                        self.history_buffer[history_ptr] = self.history_buffer[pattern_start + idx];
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
                let wrap_src = history_end - (copy_offset_usize - history_ptr) + 1;

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
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "offset bounded by 65536 (fits u16); hash from u16::from_le_bytes widens to usize"
    )]
    pub(crate) fn hash_table_add(&mut self, src_data: &[u8], src_size: usize, history_offset: usize) {
        if src_size < 8 {
            return;
        }
        let end_offset = history_offset + src_size - 8;
        let mut offset = history_offset;
        let mut src_idx = 0usize;

        while offset < end_offset {
            let hash = usize::from(u16::from_le_bytes([src_data[src_idx], src_data[src_idx + 1]]));
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
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "usize→i32: offsets bounded by 64KB history buffer, always fit in i32"
    )]
    fn find_match_length(&self, offset1: usize, offset2: usize, limit: usize) -> i32 {
        let buf = &*self.history_buffer;
        let start = offset1;
        let mut i1 = offset1;
        let mut i2 = offset2;

        // Fast path: compare 8 bytes at a time using u64 XOR.
        while i1 + 8 <= limit && i2 + 8 < buf.len() {
            let a = u64::from_ne_bytes(buf[i1..i1 + 8].try_into().unwrap_or_else(|_| unreachable!()));
            let b = u64::from_ne_bytes(buf[i2..i2 + 8].try_into().unwrap_or_else(|_| unreachable!()));
            if a != b {
                let xor = a ^ b;
                let diff_byte = if cfg!(target_endian = "little") {
                    xor.trailing_zeros() / 8
                } else {
                    xor.leading_zeros() / 8
                } as usize;
                i1 += diff_byte + 1;
                return (i1 as i32) - (start as i32) - 1;
            }
            i1 += 8;
            i2 += 8;
        }

        // Slow path: byte-by-byte for remaining bytes.
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
    #[expect(
        clippy::as_conversions,
        clippy::cast_sign_loss,
        reason = "i32→usize: find_match_length returns i32 bounded by 64KB buffer; \
                  u16 offsets widen to usize for array indexing"
    )]
    pub(crate) fn find_best_match(&mut self, history_offset: u16) -> Result<Option<(usize, u16)>, BulkError> {
        let ho = usize::from(history_offset);

        if self.match_table[ho] == 0 {
            return Ok(None);
        }

        let mut match_length: usize = 2;
        let mut offset: u16 = history_offset;
        let history_ptr = self.history_offset; // end of valid data

        // Sentinel: allows the chain-following logic to work at position 0
        self.match_table[0] = history_offset;
        let mut match_offset: u16 = self.match_table[ho];
        let mut next_offset: u16 = self.match_table[usize::from(offset)];

        for _i in 0..4 {
            let mut j: i32 = -1;

            // 6 chain-following steps with quick-filter check.
            // Each step follows the chain one link and checks if the
            // candidate's byte at position `match_length` matches the
            // current position's byte at `history_offset + match_length`.
            // Alternates between Offset and NextOffset.
            let target_byte = self.history_buffer[ho + match_length];

            if j < 0 {
                offset = self.match_table[usize::from(next_offset)];
                if self.history_buffer[match_length + usize::from(next_offset)] == target_byte {
                    j = 0;
                }
            }
            if j < 0 {
                next_offset = self.match_table[usize::from(offset)];
                if self.history_buffer[match_length + usize::from(offset)] == target_byte {
                    j = 1;
                }
            }
            if j < 0 {
                offset = self.match_table[usize::from(next_offset)];
                if self.history_buffer[match_length + usize::from(next_offset)] == target_byte {
                    j = 2;
                }
            }
            if j < 0 {
                next_offset = self.match_table[usize::from(offset)];
                if self.history_buffer[match_length + usize::from(offset)] == target_byte {
                    j = 3;
                }
            }
            if j < 0 {
                offset = self.match_table[usize::from(next_offset)];
                if self.history_buffer[match_length + usize::from(next_offset)] == target_byte {
                    j = 4;
                }
            }
            if j < 0 {
                next_offset = self.match_table[usize::from(offset)];
                if self.history_buffer[match_length + usize::from(offset)] == target_byte {
                    j = 5;
                }
            }

            if j >= 0 {
                // Pick the candidate: even j → NextOffset, odd j → Offset
                if (j % 2) == 0 {
                    offset = next_offset;
                }

                if (offset != history_offset) && (offset != 0) {
                    let len = self.find_match_length(ho + 2, usize::from(offset) + 2, history_ptr);
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
                        next_offset = self.match_table[usize::from(offset)];
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
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "history_ptr bounded by 65536; i32 arithmetic for offset adjustment; \
                  hash/match table entries are u16 (< 65536)"
    )]
    pub(crate) fn move_encoder_windows(&mut self, history_ptr: usize) -> Result<(), BulkError> {
        const HALF: usize = HISTORY_BUFFER_SIZE / 2; // 32768

        if !(HALF..=HISTORY_BUFFER_SIZE).contains(&history_ptr) {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH: invalid history ptr for window move",
            ));
        }

        // Move last 32 KB to front
        self.history_buffer.copy_within((history_ptr - HALF)..history_ptr, 0);

        let history_offset = (history_ptr - HALF) as i32;

        // Adjust hash table entries: subtract the offset shift
        for entry in self.hash_table.iter_mut() {
            let new_val = i32::from(*entry) - history_offset;
            *entry = if new_val <= 0 { 0 } else { new_val as u16 };
        }

        // Adjust match table entries (relocate first half)
        const MATCH_HALF: usize = MATCH_TABLE_SIZE / 2;
        for j in 0..MATCH_HALF {
            let src_idx = (history_offset as usize) + j;
            if src_idx >= MATCH_TABLE_SIZE {
                continue;
            }
            let new_val = i32::from(self.match_table[src_idx]) - history_offset;
            self.match_table[j] = if new_val <= 0 { 0 } else { new_val as u16 };
        }

        // Zero upper half of match table
        self.match_table[MATCH_HALF..MATCH_TABLE_SIZE].fill(0);

        Ok(())
    }

    // ---------------------------------------------------------------
    // Huffman encoding helpers for NCRUSH compression
    // ---------------------------------------------------------------

    /// Reads a little-endian 16-bit Huffman code from the `HuffCodeLEC` byte
    /// array at the given symbol index.
    ///
    /// `HuffCodeLEC` stores codes as pairs of bytes (LE). For symbol `index`,
    /// the two bytes at `[2*index]` and `[2*index + 1]` form the 16-bit code.
    fn get_lec_code(index: usize) -> Result<u32, BulkError> {
        let byte_index = index * 2;
        if byte_index + 1 >= tables::HuffCodeLEC.len() {
            return Err(BulkError::InvalidCompressedData("HuffCodeLEC index out of bounds"));
        }
        let lo = u32::from(tables::HuffCodeLEC[byte_index]);
        let hi = u32::from(tables::HuffCodeLEC[byte_index + 1]);
        Ok(lo | (hi << 8))
    }

    /// Encodes a literal byte using the LEC Huffman table.
    ///
    /// Writes `HuffLengthLEC[literal]` bits of `HuffCodeLEC[2*literal]` (LE word).
    ///
    /// Ported from FreeRDP's literal encoding in `ncrush_compress`.
    pub(crate) fn encode_literal(writer: &mut NCrushBitWriter<'_>, literal: u8) -> Result<(), BulkError> {
        let index = usize::from(literal);
        if index >= tables::HuffLengthLEC.len() {
            return Err(BulkError::InvalidCompressedData(
                "Literal index out of HuffLengthLEC range",
            ));
        }
        let bit_length = u32::from(tables::HuffLengthLEC[index]);
        if bit_length > 15 {
            return Err(BulkError::InvalidCompressedData(
                "Literal Huffman code length exceeds 15",
            ));
        }
        let code = Self::get_lec_code(index)?;
        writer.write_bits(code, bit_length)
    }

    /// Encodes a CopyOffset that is **not** in the offset cache.
    ///
    /// 1. Looks up the copy-offset index via `huff_table_copy_offset`.
    /// 2. Writes the Huffman code for `LEC[257 + copy_offset_index]`.
    /// 3. Writes the extra low-order bits of the raw copy-offset.
    ///
    /// Ported from FreeRDP's non-cache CopyOffset encoding in `ncrush_compress`.
    #[expect(
        clippy::as_conversions,
        reason = "copy_offset >> 7 + 256 bounded by table size; lookup_idx usize for indexing"
    )]
    pub(crate) fn encode_copy_offset(
        &self,
        writer: &mut NCrushBitWriter<'_>,
        copy_offset: u32,
    ) -> Result<(), BulkError> {
        // Map raw offset to lookup index (same as FreeRDP)
        let lookup = if copy_offset >= 256 {
            (copy_offset >> 7) + 256
        } else {
            copy_offset
        };

        let lookup_idx = (lookup as usize) + 2; // +2 matches FreeRDP's `bits + 2`
        if lookup_idx >= HUFF_TABLE_COPY_OFFSET_SIZE {
            return Err(BulkError::InvalidCompressedData("CopyOffset lookup index out of range"));
        }

        let copy_offset_index = usize::from(self.huff_table_copy_offset[lookup_idx]);

        if copy_offset_index >= tables::CopyOffsetBitsLUT.len() {
            return Err(BulkError::InvalidCompressedData(
                "CopyOffsetIndex out of CopyOffsetBitsLUT range",
            ));
        }
        let copy_offset_bits = tables::CopyOffsetBitsLUT[copy_offset_index];

        let index_lec = 257 + copy_offset_index;
        if index_lec >= tables::HuffLengthLEC.len() {
            return Err(BulkError::InvalidCompressedData(
                "CopyOffset LEC index out of HuffLengthLEC range",
            ));
        }
        let bit_length = u32::from(tables::HuffLengthLEC[index_lec]);
        if bit_length > 15 {
            return Err(BulkError::InvalidCompressedData(
                "CopyOffset Huffman code length exceeds 15",
            ));
        }
        if copy_offset_bits > 18 {
            return Err(BulkError::InvalidCompressedData("CopyOffset extra bits exceed 18"));
        }

        let code = Self::get_lec_code(index_lec)?;
        writer.write_bits(code, bit_length)?;

        // Write extra bits (the low-order bits of the raw offset)
        if copy_offset_bits > 0 {
            let mask = (1u32 << copy_offset_bits) - 1;
            let masked_bits = copy_offset & mask;
            writer.write_bits(masked_bits, copy_offset_bits)?;
        }

        Ok(())
    }

    /// Encodes an offset-cache hit (CopyOffset found in the LRU cache).
    ///
    /// Writes the Huffman code for `LEC[289 + cache_index]`.
    ///
    /// Ported from FreeRDP's OffsetCache encoding in `ncrush_compress`.
    pub(crate) fn encode_offset_cache_hit(
        writer: &mut NCrushBitWriter<'_>,
        cache_index: usize,
    ) -> Result<(), BulkError> {
        let index_lec = 289 + cache_index;
        if index_lec >= tables::HuffLengthLEC.len() {
            return Err(BulkError::InvalidCompressedData(
                "OffsetCache LEC index out of HuffLengthLEC range",
            ));
        }
        let bit_length = u32::from(tables::HuffLengthLEC[index_lec]);
        if bit_length >= 15 {
            return Err(BulkError::InvalidCompressedData(
                "OffsetCache Huffman code length >= 15",
            ));
        }
        let code = Self::get_lec_code(index_lec)?;
        writer.write_bits(code, bit_length)
    }

    /// Encodes a match length using the LOM Huffman table.
    ///
    /// 1. Looks up `IndexCO` via `huff_table_lom` (or uses 28 for large lengths).
    /// 2. Writes `HuffCodeLOM[IndexCO]` with `HuffLengthLOM[IndexCO]` bits.
    /// 3. Writes extra bits for the difference from `LOMBaseLUT[IndexCO]`.
    ///
    /// The `match_length` parameter is the **raw** match length (not minus 2).
    /// FreeRDP uses `(MatchLength - 2)` for the LOM table lookup but keeps
    /// `MatchLength` for the extra-bits calculation.
    ///
    /// Ported from FreeRDP's LOM encoding in `ncrush_compress`.
    #[expect(
        clippy::as_conversions,
        reason = "match_length bounded by 4096 (fits usize); huff_table_lom entries are u8→usize"
    )]
    pub(crate) fn encode_length_of_match(
        &self,
        writer: &mut NCrushBitWriter<'_>,
        match_length: u32,
    ) -> Result<(), BulkError> {
        // FreeRDP: if ((MatchLength - 2) >= 768) IndexCO = 28; else IndexCO = HuffTableLOM[MatchLength];
        let index_co = if (match_length.wrapping_sub(2)) >= 768 {
            28usize
        } else {
            if (match_length as usize) >= HUFF_TABLE_LOM_SIZE {
                return Err(BulkError::InvalidCompressedData(
                    "MatchLength out of HuffTableLOM range",
                ));
            }
            usize::from(self.huff_table_lom[match_length as usize])
        };

        if index_co >= tables::HuffLengthLOM.len() {
            return Err(BulkError::InvalidCompressedData(
                "LOM IndexCO out of HuffLengthLOM range",
            ));
        }
        let bit_length = u32::from(tables::HuffLengthLOM[index_co]);

        if index_co >= tables::LOMBitsLUT.len() {
            return Err(BulkError::InvalidCompressedData("LOM IndexCO out of LOMBitsLUT range"));
        }
        let lom_bits = tables::LOMBitsLUT[index_co];

        if index_co >= tables::HuffCodeLOM.len() {
            return Err(BulkError::InvalidCompressedData("LOM IndexCO out of HuffCodeLOM range"));
        }
        writer.write_bits(u32::from(tables::HuffCodeLOM[index_co]), bit_length)?;

        // Write extra bits: (MatchLength - 2) & mask
        if lom_bits > 0 {
            let mask = (1u32 << lom_bits) - 1;
            let masked_bits = match_length.wrapping_sub(2) & mask;

            // Verify the encoding is consistent
            if index_co >= tables::LOMBaseLUT.len() {
                return Err(BulkError::InvalidCompressedData("LOM IndexCO out of LOMBaseLUT range"));
            }
            if masked_bits + tables::LOMBaseLUT[index_co] != match_length {
                return Err(BulkError::InvalidCompressedData(
                    "LOM encoding inconsistency: MaskedBits + LOMBase != MatchLength",
                ));
            }

            writer.write_bits(masked_bits, lom_bits)?;
        }

        Ok(())
    }

    /// Encodes the end-of-stream marker (symbol 256 in the LEC table).
    ///
    /// Ported from FreeRDP's EOS encoding at the end of `ncrush_compress`.
    pub(crate) fn encode_eos(writer: &mut NCrushBitWriter<'_>) -> Result<(), BulkError> {
        let index = 256;
        if index >= tables::HuffLengthLEC.len() {
            return Err(BulkError::InvalidCompressedData("EOS index out of HuffLengthLEC range"));
        }
        let bit_length = u32::from(tables::HuffLengthLEC[index]);
        if bit_length > 15 {
            return Err(BulkError::InvalidCompressedData("EOS Huffman code length exceeds 15"));
        }
        let code = Self::get_lec_code(index)?;
        writer.write_bits(code, bit_length)
    }

    // ---------------------------------------------------------------
    // NCRUSH compress
    // ---------------------------------------------------------------

    /// Compresses `src_data` using the NCRUSH algorithm.
    ///
    /// `dst_buffer` must be at least `src_data.len()` bytes.
    ///
    /// On success returns `(compressed_size, flags)`.
    /// - If `flags & PACKET_COMPRESSED != 0`: the compressed data is in
    ///   `dst_buffer[..compressed_size]`.
    /// - If `flags & PACKET_FLUSHED != 0` **and** `flags & PACKET_COMPRESSED == 0`:
    ///   compression was abandoned (output would exceed input); the caller
    ///   should transmit the original `src_data` uncompressed. The context
    ///   has been reset.
    ///
    /// Ported from FreeRDP's `ncrush_compress`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "history offsets bounded by 65536 (fit u16/u32); \
                  copy_offset bounded by history_buffer_size-1 (fits u32); \
                  match_length bounded by history buffer (fits u32)"
    )]
    pub(crate) fn compress(&mut self, src_data: &[u8], dst_buffer: &mut [u8]) -> Result<(usize, u32), BulkError> {
        use crate::flags;

        const COMPRESSION_LEVEL: u32 = 2; // NCRUSH compression type

        let src_size = src_data.len();
        if src_size == 0 {
            return Ok((0, COMPRESSION_LEVEL));
        }

        let mut out_flags: u32 = 0;
        let mut packet_at_front = false;
        let mut packet_flushed = false;

        // --- Window management: check if we need to slide or flush ---
        // FreeRDP: if ((SrcSize + ncrush->HistoryOffset) >= 65529)
        if src_size + self.history_offset >= 65529 {
            if self.history_offset == self.history_buffer_size + 1 {
                // Previously flushed — reset offset
                self.history_offset = 0;
                packet_flushed = true;
            } else {
                // Slide the encoder window
                self.move_encoder_windows(self.history_offset)?;
                self.history_offset = 32768;
                packet_at_front = true;
            }
        }

        if dst_buffer.len() < src_size {
            return Err(BulkError::OutputBufferTooSmall {
                required: src_size,
                available: dst_buffer.len(),
            });
        }

        let _dst_size = src_size; // Compressed output must not exceed source size

        // --- Populate hash chains and copy source into history buffer ---
        let history_offset = self.history_offset;
        self.hash_table_add(src_data, src_size, history_offset);

        // Copy source data into the history buffer at the current offset
        let hist_end = history_offset + src_size;
        if hist_end > HISTORY_BUFFER_SIZE {
            return Err(BulkError::HistoryBufferOverflow);
        }
        self.history_buffer[history_offset..hist_end].copy_from_slice(src_data);
        let history_ptr_limit = hist_end; // End of valid data (for bounds check)

        // Set history_offset to end of valid data — find_best_match reads
        // self.history_offset as the limit for find_match_length.
        // (FreeRDP: ncrush->HistoryPtr = &HistoryPtr[SrcSize])
        self.history_offset = hist_end;

        // --- Main compression loop ---
        let mut writer = NCrushBitWriter::new(dst_buffer);
        let mut src_pos: usize = 0;
        let mut history_ptr: usize = history_offset; // Current position in history buffer

        // Process all bytes except the last 2 (match needs at least 2 bytes ahead)
        while src_pos < src_size.saturating_sub(2) {
            let mut match_length: usize = 0;
            let ho = history_ptr;

            // Bounds check (FreeRDP: HistoryPtr > ncrush->HistoryPtr)
            if ho > history_ptr_limit {
                return Err(BulkError::InvalidCompressedData(
                    "NCRUSH compress: history pointer past limit",
                ));
            }
            if ho >= HISTORY_BUFFER_SIZE {
                return Err(BulkError::InvalidCompressedData(
                    "NCRUSH compress: history offset >= 65536",
                ));
            }

            // Try to find a match via the hash chain
            let mut match_offset: u16 = 0;
            if self.match_table[ho] != 0 {
                if let Some((mlen, moff)) = self.find_best_match(ho as u16)? {
                    match_length = mlen;
                    match_offset = moff;
                }
            }

            // Compute CopyOffset if we found a match
            let copy_offset = if match_length > 0 {
                let match_offset_usize = usize::from(match_offset);
                let dist = if history_ptr >= match_offset_usize {
                    history_ptr - match_offset_usize
                } else {
                    // Wrap around
                    history_ptr + HISTORY_BUFFER_SIZE - match_offset_usize
                };
                (self.history_buffer_size - 1) & dist
            } else {
                0
            };

            // FreeRDP: discard 2-byte match if offset >= 64
            if match_length == 2 && copy_offset >= 64 {
                match_length = 0;
            }

            if match_length == 0 {
                // --- Encode literal ---
                let literal = src_data[src_pos];
                src_pos += 1;
                history_ptr += 1;

                // Check output space (PACKET_FLUSH #1)
                if writer.would_overflow(2) {
                    self.reset(true);
                    return Ok((src_size, flags::PACKET_FLUSHED | COMPRESSION_LEVEL));
                }

                Self::encode_literal(&mut writer, literal)?;
            } else {
                // --- Encode match ---
                history_ptr += match_length;
                src_pos += match_length;

                // Check output space (PACKET_FLUSH #2)
                if writer.would_overflow(8) {
                    self.reset(true);
                    return Ok((src_size, flags::PACKET_FLUSHED | COMPRESSION_LEVEL));
                }

                // --- Offset cache management (LRU) ---
                let mut offset_cache_index: usize = 5; // sentinel: not in cache

                // copy_offset is bounded by (history_buffer_size - 1) = 65535, fits in u32
                let copy_offset_u32 = copy_offset as u32;

                if copy_offset_u32 == self.offset_cache[0]
                    || copy_offset_u32 == self.offset_cache[1]
                    || copy_offset_u32 == self.offset_cache[2]
                    || copy_offset_u32 == self.offset_cache[3]
                {
                    if copy_offset_u32 == self.offset_cache[3] {
                        self.offset_cache.swap(3, 0);
                        offset_cache_index = 3;
                    } else if copy_offset_u32 == self.offset_cache[2] {
                        self.offset_cache.swap(2, 0);
                        offset_cache_index = 2;
                    } else if copy_offset_u32 == self.offset_cache[1] {
                        self.offset_cache.swap(1, 0);
                        offset_cache_index = 1;
                    } else {
                        // copy_offset_u32 == self.offset_cache[0]
                        offset_cache_index = 0;
                    }
                } else {
                    // Not in cache — push new offset, shift others down
                    self.offset_cache[3] = self.offset_cache[2];
                    self.offset_cache[2] = self.offset_cache[1];
                    self.offset_cache[1] = self.offset_cache[0];
                    self.offset_cache[0] = copy_offset_u32;
                }

                let match_length_u32 = match_length as u32;

                if offset_cache_index >= 4 {
                    // CopyOffset NOT in cache
                    self.encode_copy_offset(&mut writer, copy_offset_u32)?;
                    self.encode_length_of_match(&mut writer, match_length_u32)?;
                } else {
                    // CopyOffset IS in cache
                    Self::encode_offset_cache_hit(&mut writer, offset_cache_index)?;
                    self.encode_length_of_match(&mut writer, match_length_u32)?;
                }
            }

            // FreeRDP: if (HistoryPtr >= HistoryBufferEndPtr) return -1013;
            if history_ptr >= HISTORY_BUFFER_SIZE {
                return Err(BulkError::InvalidCompressedData(
                    "NCRUSH compress: history pointer reached buffer end",
                ));
            }
        }

        // --- Encode remaining trailing literals (last 0-2 bytes) ---
        while src_pos < src_size {
            // Check output space (PACKET_FLUSH #3)
            if writer.would_overflow(2) {
                self.reset(true);
                return Ok((src_size, flags::PACKET_FLUSHED | COMPRESSION_LEVEL));
            }

            let literal = src_data[src_pos];
            src_pos += 1;
            history_ptr += 1;

            Self::encode_literal(&mut writer, literal)?;
        }

        // --- Check output space for EOS + finish (PACKET_FLUSH #4) ---
        if writer.would_overflow(4) {
            self.reset(true);
            return Ok((src_size, flags::PACKET_FLUSHED | COMPRESSION_LEVEL));
        }

        // --- Encode end-of-stream marker ---
        Self::encode_eos(&mut writer)?;
        writer.finish()?;

        let compressed_size = writer.bytes_written();

        // If compressed output is larger than source, flush
        if compressed_size > src_size {
            self.reset(true);
            return Ok((src_size, flags::PACKET_FLUSHED | COMPRESSION_LEVEL));
        }

        // --- Build flags ---
        out_flags |= flags::PACKET_COMPRESSED;
        out_flags |= COMPRESSION_LEVEL;

        if packet_at_front {
            out_flags |= flags::PACKET_AT_FRONT;
        }

        if packet_flushed {
            out_flags |= flags::PACKET_FLUSHED;
        }

        // Update history offset for next call
        self.history_offset = history_ptr;

        if self.history_offset >= self.history_buffer_size {
            return Err(BulkError::InvalidCompressedData(
                "NCRUSH compress: final history offset out of range",
            ));
        }

        Ok((compressed_size, out_flags))
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
    fn test_ncrush_context_new() {
        let ctx = NCrushContext::new().unwrap();
        assert_eq!(ctx.history_buffer_size, HISTORY_BUFFER_SIZE);
        assert_eq!(ctx.history_end_offset, HISTORY_BUFFER_SIZE - 1);
        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.history_buffer_fence, HISTORY_BUFFER_FENCE);
        assert_eq!(ctx.offset_cache, [0u32; 4]);
    }

    #[test]
    fn test_ncrush_context_reset_no_flush() {
        let mut ctx = NCrushContext::new().unwrap();
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
        let mut ctx = NCrushContext::new().unwrap();
        ctx.reset(true);

        assert_eq!(ctx.history_offset, HISTORY_BUFFER_SIZE + 1);
    }

    #[test]
    fn test_ncrush_generate_tables_lom() {
        let ctx = NCrushContext::new().unwrap();

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
        let ctx = NCrushContext::new().unwrap();

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

        let mut ctx = NCrushContext::new().unwrap();
        let data = b"hello world";

        // No PACKET_COMPRESSED flag → should return source data directly
        let result = ctx.decompress(data, flags::PACKET_FLUSHED).unwrap();
        assert_eq!(result, b"hello world");
        // History offset should remain 0 (no decompression occurred)
        assert_eq!(ctx.history_offset, 0);
    }

    #[test]
    fn test_ncrush_decompress_flushed_clears_state() {
        use crate::flags;

        let mut ctx = NCrushContext::new().unwrap();
        ctx.history_offset = 1000;
        ctx.offset_cache[0] = 42;
        ctx.history_buffer[500] = 0xFF;

        let data = b"test";
        let _result = ctx.decompress(data, flags::PACKET_FLUSHED).unwrap();

        // PACKET_FLUSHED should clear history and offset cache
        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.offset_cache, [0u32; 4]);
        assert_eq!(ctx.history_buffer[500], 0);
    }

    #[test]
    fn test_ncrush_decompress_compressed_too_short() {
        use crate::flags;

        let mut ctx = NCrushContext::new().unwrap();
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
        assert!(NCrushContext::fetch_bits(&src, &mut src_pos2, &mut nbits2, &mut bits2));
        assert_eq!(nbits2, 20); // unchanged
        assert_eq!(bits2, 0x12345); // unchanged
        assert_eq!(src_pos2, 0); // no bytes consumed

        // nbits < 16, fetch 2 bytes
        assert!(NCrushContext::fetch_bits(&src, &mut src_pos, &mut nbits, &mut bits));
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

        assert!(NCrushContext::fetch_bits(&src, &mut src_pos, &mut nbits, &mut bits));
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
        assert!(NCrushContext::fetch_bits(&src, &mut src_pos, &mut nbits, &mut bits));
        assert_eq!(nbits, 5); // unchanged
    }

    #[test]
    fn test_ncrush_decompress_fetch_bits_exhausted_fail() {
        let src: [u8; 0] = [];
        let mut src_pos = 0usize;
        let mut nbits: i32 = -1;
        let mut bits: u32 = 0;

        // No more data and nbits < 0 → fail
        assert!(!NCrushContext::fetch_bits(&src, &mut src_pos, &mut nbits, &mut bits));
    }

    /// Byte-exact decompression test ported from FreeRDP
    /// `test_NCrushDecompressBells` in `TestFreeRDPCodecNCrush.c`.
    ///
    /// Verifies that NCRUSH decompression of the compressed "bells" data
    /// produces the original plaintext byte-for-byte.
    #[test]
    fn test_ncrush_decompress_bells() {
        use crate::flags;

        let mut ctx = NCrushContext::new().unwrap();

        // FreeRDP flags: PACKET_COMPRESSED | 2 (compression type NCRUSH)
        let flags_value = flags::PACKET_COMPRESSED | 0x02;

        let result = ctx.decompress(test_data::TEST_BELLS_NCRUSH, flags_value).unwrap();

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
        let mut ctx = NCrushContext::new().unwrap();

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
        let mut ctx = NCrushContext::new().unwrap();

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
        let mut ctx = NCrushContext::new().unwrap();

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
        let mut ctx = NCrushContext::new().unwrap();

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
        let ctx = NCrushContext::new().unwrap();

        // offset1 > limit immediately → returns -1
        let len = ctx.find_match_length(10, 20, 5);
        assert_eq!(len, -1);
    }

    #[test]
    fn test_ncrush_find_best_match_no_chain() {
        let mut ctx = NCrushContext::new().unwrap();

        // match_table[100] = 0 → no chain
        let result = ctx.find_best_match(100).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_ncrush_find_best_match_simple() {
        let mut ctx = NCrushContext::new().unwrap();

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
        let mut ctx = NCrushContext::new().unwrap();

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
        let mut ctx = NCrushContext::new().unwrap();

        // Entry pointing before the offset should be clamped to 0
        ctx.hash_table[42] = 50; // 50 < offset (say, 100)

        ctx.move_encoder_windows(32868).unwrap();
        // history_offset = 32868 - 32768 = 100
        // 50 - 100 = -50 → clamped to 0
        assert_eq!(ctx.hash_table[42], 0);
    }

    // ---------------------------------------------------------------
    // NCrushBitWriter tests
    // ---------------------------------------------------------------

    #[test]
    fn test_ncrush_bit_writer_basic() {
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // Write 8 bits: 0xAB
        writer.write_bits(0xAB, 8).unwrap();
        assert_eq!(writer.bytes_written(), 0); // not flushed yet (< 16 bits)

        // Write 8 more bits: 0xCD → accumulator has 16 bits, should flush
        writer.write_bits(0xCD, 8).unwrap();
        assert_eq!(writer.bytes_written(), 2);
        // Flushed bytes should be LE: low byte first
        assert_eq!(buf[0], 0xAB);
        assert_eq!(buf[1], 0xCD);
    }

    #[test]
    fn test_ncrush_bit_writer_finish() {
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // Write 5 bits
        writer.write_bits(0x15, 5).unwrap();
        assert_eq!(writer.bytes_written(), 0);

        writer.finish().unwrap();
        assert_eq!(writer.bytes_written(), 2);
        assert_eq!(buf[0], 0x15);
        assert_eq!(buf[1], 0x00);
    }

    #[test]
    fn test_ncrush_bit_writer_overflow() {
        let mut buf = [0u8; 2]; // Only room for one flush
        let mut writer = NCrushBitWriter::new(&mut buf);

        // Fill 16 bits → flush (2 bytes)
        writer.write_bits(0xFFFF, 16).unwrap();
        assert_eq!(writer.bytes_written(), 2);

        // Another 16 bits → should fail
        let result = writer.write_bits(0x0001, 16);
        assert!(result.is_err());
    }

    #[test]
    fn test_ncrush_bit_writer_accumulation() {
        // Verify bits are accumulated LSB-first
        let mut buf = [0u8; 4];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // Write 4 bits: 0b1010
        writer.write_bits(0b1010, 4).unwrap();
        // Write 4 bits: 0b0101 → accumulator = 0b0101_1010
        writer.write_bits(0b0101, 4).unwrap();
        // Write 8 bits: 0xFF → accumulator has 16 bits, flush
        writer.write_bits(0xFF, 8).unwrap();
        assert_eq!(writer.bytes_written(), 2);
        // Low byte: 0b0101_1010 = 0x5A, High byte: 0xFF
        assert_eq!(buf[0], 0x5A);
        assert_eq!(buf[1], 0xFF);
    }

    // ---------------------------------------------------------------
    // Huffman encoding helper tests
    // ---------------------------------------------------------------

    #[test]
    fn test_ncrush_encode_literal() {
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // Encode literal 0 (space character equivalent in many codings)
        NCrushContext::encode_literal(&mut writer, 0).unwrap();

        // HuffLengthLEC[0] = 6, HuffCodeLEC[0..2] = [0x04, 0x00] → code = 0x0004
        // After write_bits(0x0004, 6): accumulator = 0x04, offset = 6
        // Not flushed yet — finish to see the output
        writer.finish().unwrap();
        assert_eq!(buf[0], 0x04);
        assert_eq!(buf[1], 0x00);
    }

    #[test]
    fn test_ncrush_encode_two_literals() {
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // Encode literal 0: code=0x04, len=6
        NCrushContext::encode_literal(&mut writer, 0).unwrap();
        // Encode literal 1: code=0x24, len=6
        NCrushContext::encode_literal(&mut writer, 1).unwrap();
        // Total: 12 bits — not flushed yet

        // Encode literal 2: code=0x14, len=6
        NCrushContext::encode_literal(&mut writer, 2).unwrap();
        // Total: 18 bits → should have flushed 16 bits

        assert_eq!(writer.bytes_written(), 2);

        // accumulator after 3 writes:
        // bits 0-5:   0x04 = 0b000100
        // bits 6-11:  0x24 = 0b100100
        // bits 12-17: 0x14 = 0b010100
        // Combined: 0b010100_100100_000100
        // Lower 16 bits: 0b0100_100100_000100 = 0x4904
        assert_eq!(buf[0], 0x04);
        assert_eq!(buf[1], 0x49);
    }

    #[test]
    fn test_ncrush_encode_eos() {
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // EOS is symbol 256 in LEC table
        NCrushContext::encode_eos(&mut writer).unwrap();

        // HuffLengthLEC[256] = 13, HuffCodeLEC[512..514] = [0xFF, 0x17] → code = 0x17FF
        writer.finish().unwrap();
        // 13 bits of 0x17FF = lower 13 bits = 0x17FF & 0x1FFF = 0x17FF
        assert_eq!(buf[0], 0xFF);
        assert_eq!(buf[1], 0x17);
    }

    #[test]
    fn test_ncrush_encode_offset_cache_hit() {
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // Cache index 0 → LEC index 289
        // HuffLengthLEC[289] = 5, HuffCodeLEC[578..580] = [0x18, 0x00] → code = 0x0018
        NCrushContext::encode_offset_cache_hit(&mut writer, 0).unwrap();
        writer.finish().unwrap();
        assert_eq!(buf[0], 0x18);
        assert_eq!(buf[1], 0x00);
    }

    #[test]
    fn test_ncrush_encode_length_of_match_simple() {
        let ctx = NCrushContext::new().unwrap();
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // match_length = 2 (minimum match)
        // huff_table_lom[2] = 0 → IndexCO = 0
        // HuffLengthLOM[0] = 4, HuffCodeLOM[0] = 0x0001
        // LOMBitsLUT[0] = 0 → no extra bits
        ctx.encode_length_of_match(&mut writer, 2).unwrap();
        writer.finish().unwrap();
        assert_eq!(buf[0], 0x01);
        assert_eq!(buf[1], 0x00);
    }

    #[test]
    fn test_ncrush_encode_copy_offset_small() {
        let ctx = NCrushContext::new().unwrap();
        let mut buf = [0u8; 16];
        let mut writer = NCrushBitWriter::new(&mut buf);

        // CopyOffset = 1 (small offset)
        // lookup = 1, lookup_idx = 3
        // huff_table_copy_offset[3] should be 1 (from generate_tables)
        // CopyOffsetBitsLUT[1] = 0 → no extra bits
        // IndexLEC = 257 + 1 = 258
        // The encoding should succeed without error
        ctx.encode_copy_offset(&mut writer, 1).unwrap();
        writer.finish().unwrap();

        // Just verify it wrote something without error
        assert!(writer.bytes_written() > 0);
    }

    #[test]
    fn test_ncrush_encode_would_overflow() {
        let writer = NCrushBitWriter::new(&mut []);
        assert!(writer.would_overflow(1));
    }

    // ---------------------------------------------------------------
    // ncrush_compress tests
    // ---------------------------------------------------------------

    #[test]
    fn test_ncrush_compress_basic() {
        let mut ctx = NCrushContext::new().unwrap();
        let data = b"hello world";
        let mut dst = vec![0u8; 256];

        let (size, flags_out) = ctx.compress(data, &mut dst).unwrap();

        // Should produce compressed output (or flush if output > src)
        // Either way, it should not error
        assert!(size > 0);
        // flags should include COMPRESSION_LEVEL (2)
        assert_ne!(flags_out & 0x0F, 0); // compression type != 0
    }

    #[test]
    fn test_ncrush_compress_with_repeats() {
        let mut ctx = NCrushContext::new().unwrap();
        // Repetitive data should compress well
        let data = b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let mut dst = vec![0u8; 256];

        let (size, flags_out) = ctx.compress(data, &mut dst).unwrap();
        assert!(size > 0);

        // With enough repetition, compression should succeed
        if flags_out & crate::flags::PACKET_COMPRESSED != 0 {
            assert!(size < data.len());
        }
    }

    #[test]
    fn test_ncrush_compress_empty() {
        let mut ctx = NCrushContext::new().unwrap();
        let data = b"";
        let mut dst = vec![0u8; 256];

        let (size, flags_out) = ctx.compress(data, &mut dst).unwrap();
        assert_eq!(size, 0);
        assert_eq!(flags_out, 2); // Just compression level
    }

    #[test]
    fn test_ncrush_compress_updates_history_offset() {
        let mut ctx = NCrushContext::new().unwrap();
        let data = b"some test data for ncrush compression";
        let mut dst = vec![0u8; 256];

        let initial_offset = ctx.history_offset;
        let (_size, flags_out) = ctx.compress(data, &mut dst).unwrap();

        if flags_out & crate::flags::PACKET_COMPRESSED != 0 {
            // History offset should have advanced by the source data length
            assert_eq!(ctx.history_offset, initial_offset + data.len());
        }
    }

    #[test]
    fn test_ncrush_compress_offset_cache_updated() {
        let mut ctx = NCrushContext::new().unwrap();
        // Use data with a repeated pattern to trigger back-references
        let data = b"for.whom.the.bell.tolls,.the.bell.tolls.for.thee!";
        let mut dst = vec![0u8; 256];

        let (_size, flags_out) = ctx.compress(data, &mut dst).unwrap();

        if flags_out & crate::flags::PACKET_COMPRESSED != 0 {
            // If compression succeeded, at least one offset cache entry
            // should be non-zero (from back-references)
            let any_cached = ctx.offset_cache.iter().any(|&x| x != 0);
            assert!(any_cached, "Offset cache should have been updated");
        }
    }

    /// Byte-exact compression test ported from FreeRDP's
    /// `test_NCrushCompressBells` in `TestFreeRDPCodecNCrush.c`.
    ///
    /// Compresses the "bells" test string with a fresh compressor context
    /// and verifies the output matches FreeRDP's expected compressed bytes
    /// exactly.
    #[test]
    fn test_ncrush_compress_bells() {
        let mut ctx = NCrushContext::new().unwrap();
        let mut dst = vec![0u8; 65536];

        let (size, flags_out) = ctx.compress(test_data::TEST_BELLS_DATA, &mut dst).unwrap();

        // Must be compressed
        assert_ne!(
            flags_out & crate::flags::PACKET_COMPRESSED,
            0,
            "Expected PACKET_COMPRESSED flag, got flags: {flags_out:#010x}"
        );

        // Size must match expected
        assert_eq!(
            size,
            test_data::TEST_BELLS_NCRUSH.len(),
            "Compressed size mismatch: got {size}, expected {}",
            test_data::TEST_BELLS_NCRUSH.len()
        );

        // Content must match byte-for-byte
        assert_eq!(
            &dst[..size],
            test_data::TEST_BELLS_NCRUSH,
            "Compressed output does not match FreeRDP expected bytes"
        );
    }

    // ---------------------------------------------------------------
    // NCRUSH round-trip tests
    // ---------------------------------------------------------------

    /// Round-trip test with the FreeRDP "bells" test data.
    /// Compress → decompress → verify output matches original input.
    #[test]
    fn test_ncrush_roundtrip_bells() {
        let mut compressor = NCrushContext::new().unwrap();
        let mut decompressor = NCrushContext::new().unwrap();

        let input = test_data::TEST_BELLS_DATA;
        let mut compressed = vec![0u8; 65536];

        // Compress
        let (comp_size, flags_out) = compressor.compress(input, &mut compressed).unwrap();
        assert_ne!(
            flags_out & crate::flags::PACKET_COMPRESSED,
            0,
            "Expected compression to succeed"
        );

        // Decompress
        let decompressed = decompressor.decompress(&compressed[..comp_size], flags_out).unwrap();

        // Verify byte-for-byte match
        assert_eq!(
            decompressed, input,
            "Round-trip failed: decompressed output does not match original input"
        );
    }

    /// Round-trip test with a short repetitive pattern.
    #[test]
    fn test_ncrush_roundtrip_repetitive() {
        let mut compressor = NCrushContext::new().unwrap();
        let mut decompressor = NCrushContext::new().unwrap();

        let input = b"ABCABCABCABCABCABCABCABCABCABCABCABC";
        let mut compressed = vec![0u8; 65536];

        let (comp_size, flags_out) = compressor.compress(input, &mut compressed).unwrap();

        if flags_out & crate::flags::PACKET_COMPRESSED != 0 {
            let decompressed = decompressor.decompress(&compressed[..comp_size], flags_out).unwrap();
            assert_eq!(decompressed, &input[..]);
        }
    }

    /// Round-trip test with a longer text block containing varied content.
    #[test]
    fn test_ncrush_roundtrip_prose() {
        let mut compressor = NCrushContext::new().unwrap();
        let mut decompressor = NCrushContext::new().unwrap();

        let input = b"The quick brown fox jumps over the lazy dog. \
                       The quick brown fox jumps over the lazy dog again. \
                       And once more, the quick brown fox jumps.";
        let mut compressed = vec![0u8; 65536];

        let (comp_size, flags_out) = compressor.compress(input, &mut compressed).unwrap();

        if flags_out & crate::flags::PACKET_COMPRESSED != 0 {
            let decompressed = decompressor.decompress(&compressed[..comp_size], flags_out).unwrap();
            assert_eq!(decompressed, &input[..]);
        }
    }

    /// Round-trip test with binary-like data (all byte values 0-255).
    #[test]
    fn test_ncrush_roundtrip_binary() {
        let mut compressor = NCrushContext::new().unwrap();
        let mut decompressor = NCrushContext::new().unwrap();

        // Create a pattern with all 256 byte values repeated
        let mut input = Vec::new();
        for _ in 0..2 {
            for b in 0u8..=255 {
                input.push(b);
            }
        }
        let mut compressed = vec![0u8; 65536];

        let (comp_size, flags_out) = compressor.compress(&input, &mut compressed).unwrap();

        if flags_out & crate::flags::PACKET_COMPRESSED != 0 {
            let decompressed = decompressor.decompress(&compressed[..comp_size], flags_out).unwrap();
            assert_eq!(decompressed, &input[..]);
        }
    }

    /// Round-trip test with multiple sequential compressions on the same context
    /// (tests that history buffer state carries across calls).
    #[test]
    fn test_ncrush_roundtrip_sequential() {
        let mut compressor = NCrushContext::new().unwrap();
        let mut decompressor = NCrushContext::new().unwrap();

        let inputs: &[&[u8]] = &[
            b"first.message.to.compress",
            b"second.message.with.some.overlap.to.compress",
            b"third.message.compress.compress.compress",
        ];

        for input in inputs {
            let mut compressed = vec![0u8; 65536];

            let (comp_size, flags_out) = compressor.compress(input, &mut compressed).unwrap();

            if flags_out & crate::flags::PACKET_COMPRESSED != 0 {
                let decompressed = decompressor.decompress(&compressed[..comp_size], flags_out).unwrap();
                assert_eq!(
                    decompressed,
                    *input,
                    "Sequential round-trip failed for input: {:?}",
                    core::str::from_utf8(input)
                );
            }
        }
    }
}
