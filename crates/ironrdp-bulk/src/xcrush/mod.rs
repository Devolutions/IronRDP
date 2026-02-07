//! XCRUSH (RDP 6.1) two-level compression implementation.
//!
//! Level 1 uses chunk-based matching; Level 2 uses MPPC.
//!
//! Ported from FreeRDP's `libfreerdp/codec/xcrush.c`.

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec, vec::Vec};

use crate::error::BulkError;
use crate::flags;
use crate::mppc::MppcContext;

/// History buffer size for XCRUSH (2 MB).
///
/// XCRUSH uses a much larger history buffer than MPPC (8K/64K).
pub(crate) const HISTORY_BUFFER_SIZE: usize = 2_000_000;

/// Block buffer size for XCRUSH temporary data (16 KB).
pub(crate) const BLOCK_BUFFER_SIZE: usize = 16384;

/// Maximum number of signatures tracked by the chunk computation.
pub(crate) const MAX_SIGNATURE_COUNT: usize = 1000;

/// Maximum number of chunks in the chunk table.
pub(crate) const MAX_CHUNKS: usize = 65534;

/// Size of the next-chunk lookup table (one entry per possible chunk hash).
pub(crate) const NEXT_CHUNKS_SIZE: usize = 65536;

/// Maximum number of match entries (original or optimized).
pub(crate) const MAX_MATCH_COUNT: usize = 1000;

// ---------------------------------------------------------------------------
// Helper structures
// ---------------------------------------------------------------------------

/// Information about a single match found during chunk-based matching.
///
/// Ported from FreeRDP's `XCRUSH_MATCH_INFO`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct XCrushMatchInfo {
    /// Byte offset into the history buffer where the match starts.
    pub(crate) match_offset: u32,
    /// Byte offset of the chunk that contains the match.
    pub(crate) chunk_offset: u32,
    /// Length of the matching region in bytes.
    pub(crate) match_length: u32,
}

/// A chunk descriptor in the chunk hash table.
///
/// Ported from FreeRDP's `XCRUSH_CHUNK`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct XCrushChunk {
    /// Starting offset of this chunk in the history buffer.
    pub(crate) offset: u32,
    /// Index of the next chunk entry in the chain (0 = end of chain).
    pub(crate) next: u32,
}

/// A rolling-hash signature describing one chunk.
///
/// Ported from FreeRDP's `XCRUSH_SIGNATURE`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct XCrushSignature {
    /// The rolling hash seed value at this chunk boundary.
    pub(crate) seed: u16,
    /// The size of this chunk in bytes.
    pub(crate) size: u16,
}

/// Match detail entry in an RDP 6.1 compressed data block.
///
/// Ported from FreeRDP's `RDP61_MATCH_DETAILS`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Rdp61MatchDetails {
    /// Length of the match in bytes.
    pub(crate) match_length: u16,
    /// Offset in the decompressed output where the match is placed.
    pub(crate) match_output_offset: u16,
    /// Offset in the history buffer where the matching data is found.
    pub(crate) match_history_offset: u32,
}

/// Parsed representation of an RDP 6.1 compressed data header.
///
/// Ported from FreeRDP's `RDP61_COMPRESSED_DATA`.
#[derive(Debug)]
pub(crate) struct Rdp61CompressedData<'a> {
    /// Level-1 compression flags.
    pub(crate) level1_compr_flags: u8,
    /// Level-2 compression flags (MPPC flags).
    pub(crate) level2_compr_flags: u8,
    /// Number of match detail entries.
    pub(crate) match_count: u16,
    /// Slice of match detail entries parsed from the input.
    pub(crate) match_details: &'a [u8],
    /// Remaining literal data after the match details array.
    pub(crate) literals: &'a [u8],
}

// ---------------------------------------------------------------------------
// Main XCRUSH context
// ---------------------------------------------------------------------------

/// XCRUSH compression/decompression context.
///
/// Holds a 2 MB history buffer, chunk tables, signature arrays,
/// match arrays, and an inner MPPC context for Level-2 compression.
///
/// Ported from FreeRDP's `XCRUSH_CONTEXT` struct.
pub(crate) struct XCrushContext {
    /// Whether this context is for compression (`true`) or decompression (`false`).
    #[expect(dead_code, reason = "will be used by bulk coordinator")]
    compressor: bool,
    /// Inner MPPC context (RDP5 / 64K) for Level-2 compression/decompression.
    pub(crate) mppc: MppcContext,
    /// Current write position in the history buffer.
    pub(crate) history_offset: usize,
    /// Total history buffer size (always 2,000,000).
    pub(crate) history_buffer_size: usize,
    /// 2 MB sliding-window history buffer.
    pub(crate) history_buffer: Box<[u8; HISTORY_BUFFER_SIZE]>,
    /// 16 KB temporary block buffer used during compression.
    pub(crate) block_buffer: Box<[u8; BLOCK_BUFFER_SIZE]>,
    /// Level-2 (MPPC) compression flags carried over between calls.
    pub(crate) compression_flags: u32,
    /// Current index into the signatures array.
    pub(crate) signature_index: usize,
    /// Maximum number of signatures available.
    pub(crate) signature_count: usize,
    /// Rolling-hash signatures for chunk-based matching.
    pub(crate) signatures: Box<[XCrushSignature; MAX_SIGNATURE_COUNT]>,
    /// Head index of the chunk linked list.
    pub(crate) chunk_head: u32,
    /// Tail index of the chunk linked list.
    pub(crate) chunk_tail: u32,
    /// Chunk descriptor table (indexed by chunk ID).
    pub(crate) chunks: Box<[XCrushChunk; MAX_CHUNKS]>,
    /// Next-chunk lookup table (indexed by rolling hash value).
    pub(crate) next_chunks: Box<[u16; NEXT_CHUNKS_SIZE]>,
    /// Number of original (unoptimized) match entries found.
    pub(crate) original_match_count: usize,
    /// Number of optimized match entries after filtering.
    pub(crate) optimized_match_count: usize,
    /// Original match entries found by chunk comparison.
    pub(crate) original_matches: Box<[XCrushMatchInfo; MAX_MATCH_COUNT]>,
    /// Optimized match entries after removing overlaps.
    pub(crate) optimized_matches: Box<[XCrushMatchInfo; MAX_MATCH_COUNT]>,
}

/// Allocates a zeroed `Box<[u8; N]>` on the heap without touching the stack.
///
/// Uses `vec!` to allocate on the heap, avoiding large stack frames for
/// buffers like the 2 MB XCRUSH history.
#[expect(
    clippy::unnecessary_box_returns,
    reason = "returning Box is intentional — avoids placing large arrays on the stack"
)]
fn heap_zeroed_u8_array<const N: usize>() -> Box<[u8; N]> {
    vec![0u8; N]
        .into_boxed_slice()
        .try_into()
        .expect("vec length equals array length")
}

/// Allocates a zeroed `Box<[u16; N]>` on the heap without touching the stack.
#[expect(
    clippy::unnecessary_box_returns,
    reason = "returning Box is intentional — avoids placing large arrays on the stack"
)]
fn heap_zeroed_u16_array<const N: usize>() -> Box<[u16; N]> {
    vec![0u16; N]
        .into_boxed_slice()
        .try_into()
        .expect("vec length equals array length")
}

/// Allocates a `Box<[T; N]>` filled with `T::default()` on the heap.
#[expect(
    clippy::unnecessary_box_returns,
    reason = "returning Box is intentional — avoids placing large arrays on the stack"
)]
fn heap_default_array<T: Default + Clone + core::fmt::Debug, const N: usize>() -> Box<[T; N]> {
    vec![T::default(); N]
        .into_boxed_slice()
        .try_into()
        .expect("vec length equals array length")
}

impl XCrushContext {
    /// Creates a new XCRUSH context.
    ///
    /// `compressor`: `true` if used for compression, `false` for decompression.
    ///
    /// Large buffers (2 MB history, 512 KB chunks, etc.) are allocated on the
    /// heap via `vec!` to avoid stack overflow.
    pub(crate) fn new(compressor: bool) -> Self {
        let mut ctx = Self {
            compressor,
            mppc: MppcContext::new(1, compressor), // XCRUSH always uses RDP5 MPPC
            history_offset: 0,
            history_buffer_size: HISTORY_BUFFER_SIZE,
            history_buffer: heap_zeroed_u8_array::<HISTORY_BUFFER_SIZE>(),
            block_buffer: heap_zeroed_u8_array::<BLOCK_BUFFER_SIZE>(),
            compression_flags: 0,
            signature_index: 0,
            signature_count: MAX_SIGNATURE_COUNT,
            signatures: heap_default_array::<XCrushSignature, MAX_SIGNATURE_COUNT>(),
            chunk_head: 1,
            chunk_tail: 1,
            chunks: heap_default_array::<XCrushChunk, MAX_CHUNKS>(),
            next_chunks: heap_zeroed_u16_array::<NEXT_CHUNKS_SIZE>(),
            original_match_count: 0,
            optimized_match_count: 0,
            original_matches: heap_default_array::<XCrushMatchInfo, MAX_MATCH_COUNT>(),
            optimized_matches: heap_default_array::<XCrushMatchInfo, MAX_MATCH_COUNT>(),
        };
        ctx.reset(false);
        ctx
    }

    /// Decompresses Level-1 (chunk-based matching) XCRUSH data.
    ///
    /// Parses the RDP 6.1 compressed data format: reads match count, match
    /// details array, and literal data. Reconstructs output by interleaving
    /// literal copies with history match copies.
    ///
    /// Ported from FreeRDP's `xcrush_decompress_l1`.
    ///
    /// Returns a reference to the decompressed data in the history buffer.
    #[expect(
        clippy::as_conversions,
        reason = "LE wire format parsing requires conversions from u16/u32 to usize"
    )]
    pub(crate) fn decompress_l1<'a>(
        &'a mut self,
        src_data: &[u8],
        l1_flags: u32,
    ) -> Result<&'a [u8], BulkError> {
        if src_data.is_empty() {
            return Err(BulkError::InvalidCompressedData("XCRUSH L1: empty input"));
        }

        if l1_flags & flags::L1_PACKET_AT_FRONT != 0 {
            self.history_offset = 0;
        }

        let history_buffer_size = self.history_buffer_size;
        let mut history_ptr = self.history_offset;
        let output_start = history_ptr;

        // Track current position in the literal data
        let mut literals_start: usize;

        if l1_flags & flags::L1_NO_COMPRESSION != 0 {
            // No L1 compression — entire input is literal data
            literals_start = 0;
        } else {
            if l1_flags & flags::L1_COMPRESSED == 0 {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH L1: neither compressed nor uncompressed",
                ));
            }

            if src_data.len() < 2 {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH L1: too short for match count",
                ));
            }

            let match_count =
                u16::from_le_bytes([src_data[0], src_data[1]]) as usize;

            // Each RDP61_MATCH_DETAILS entry is 8 bytes (u16 + u16 + u32)
            let match_details_end = 2 + match_count * 8;

            if match_details_end > src_data.len() {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH L1: match details exceed input",
                ));
            }

            literals_start = match_details_end;
            let mut output_offset: usize = 0;

            for i in 0..match_count {
                let d = 2 + i * 8;
                let match_length =
                    u16::from_le_bytes([src_data[d], src_data[d + 1]]) as usize;
                let match_output_offset =
                    u16::from_le_bytes([src_data[d + 2], src_data[d + 3]]) as usize;
                let match_history_offset = u32::from_le_bytes([
                    src_data[d + 4],
                    src_data[d + 5],
                    src_data[d + 6],
                    src_data[d + 7],
                ]) as usize;

                if match_output_offset < output_offset {
                    return Err(BulkError::InvalidCompressedData(
                        "XCRUSH L1: match output offset out of order",
                    ));
                }
                if match_length > history_buffer_size {
                    return Err(BulkError::InvalidCompressedData(
                        "XCRUSH L1: match length exceeds history buffer",
                    ));
                }
                if match_history_offset > history_buffer_size {
                    return Err(BulkError::InvalidCompressedData(
                        "XCRUSH L1: match history offset exceeds history buffer",
                    ));
                }

                // Copy literal bytes between the previous output position and this match
                let literal_length = match_output_offset - output_offset;

                if literal_length > history_buffer_size {
                    return Err(BulkError::InvalidCompressedData(
                        "XCRUSH L1: literal gap exceeds history buffer",
                    ));
                }

                if literal_length > 0 {
                    let literals_end = literals_start + literal_length;

                    if history_ptr + literal_length >= history_buffer_size
                        || literals_start >= src_data.len()
                        || literals_end > src_data.len()
                    {
                        return Err(BulkError::InvalidCompressedData(
                            "XCRUSH L1: literal copy out of bounds",
                        ));
                    }

                    self.history_buffer[history_ptr..history_ptr + literal_length]
                        .copy_from_slice(&src_data[literals_start..literals_end]);
                    history_ptr += literal_length;
                    literals_start = literals_end;
                    output_offset += literal_length;

                    if literals_start > src_data.len() {
                        return Err(BulkError::InvalidCompressedData(
                            "XCRUSH L1: literals past end of input",
                        ));
                    }
                }

                // Copy match data from history buffer
                if history_ptr + match_length >= history_buffer_size
                    || match_history_offset + match_length >= history_buffer_size
                {
                    return Err(BulkError::InvalidCompressedData(
                        "XCRUSH L1: match copy out of bounds",
                    ));
                }

                // Same-buffer copy (may overlap — copy_within handles this)
                self.history_buffer.copy_within(
                    match_history_offset..match_history_offset + match_length,
                    history_ptr,
                );
                output_offset += match_length;
                history_ptr += match_length;
            }
        }

        // Copy any remaining literals after all matches
        if literals_start < src_data.len() {
            let remaining = src_data.len() - literals_start;

            if history_ptr + remaining >= history_buffer_size
                || literals_start + remaining > src_data.len()
            {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH L1: trailing literal copy out of bounds",
                ));
            }

            self.history_buffer[history_ptr..history_ptr + remaining]
                .copy_from_slice(&src_data[literals_start..]);
            history_ptr += remaining;
        }

        self.history_offset = history_ptr;
        let output_end = history_ptr;

        Ok(&self.history_buffer[output_start..output_end])
    }

    /// Decompresses XCRUSH (RDP 6.1) data.
    ///
    /// Handles all flag combinations:
    /// - Level-2 (MPPC) + Level-1 decompression
    /// - Level-1 only decompression
    /// - No compression passthrough
    ///
    /// Ported from FreeRDP's `xcrush_decompress`.
    ///
    /// Returns a reference to the decompressed data.
    pub(crate) fn decompress<'a>(
        &'a mut self,
        src_data: &[u8],
        outer_flags: u32,
    ) -> Result<&'a [u8], BulkError> {
        if src_data.len() < 2 {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: input too short for L1/L2 flags",
            ));
        }

        let level1_compr_flags = u32::from(src_data[0]);
        let level2_compr_flags = u32::from(src_data[1]);
        let inner_data = &src_data[2..];

        if outer_flags & flags::PACKET_FLUSHED != 0 {
            self.history_buffer[..self.history_buffer_size].fill(0);
            self.history_offset = 0;
        }

        if level2_compr_flags & flags::PACKET_COMPRESSED == 0 {
            // No Level-2 (MPPC) compression — go straight to L1
            return self.decompress_l1(inner_data, level1_compr_flags);
        }

        // Level-2 (MPPC) decompression first
        let mppc_output = self
            .mppc
            .decompress(inner_data, level2_compr_flags)?;

        // We need to copy the MPPC output to a temporary buffer because
        // decompress_l1 borrows self mutably and the MPPC output lives
        // in self.mppc.history_buffer.
        //
        // The MPPC output is at most 64K (MPPC history buffer size).
        let mppc_output_copy: Vec<u8> = mppc_output.to_vec();

        // Level-1 decompression on the MPPC output
        self.decompress_l1(&mppc_output_copy, level1_compr_flags)
    }

    // ========================
    // Chunk computation (compression helpers)
    // ========================

    /// Computes a hash over the first `min(32, size)` bytes of `data`.
    ///
    /// Ported from FreeRDP's `xcrush_update_hash`.
    fn update_hash(data: &[u8], size: usize) -> u16 {
        debug_assert!(size >= 4);

        let (mut seed, process_size) = if size > 32 {
            (5413u16, 32usize)
        } else {
            (5381u16, size)
        };

        let end = process_size.saturating_sub(4);
        let mut i = 0;
        while i < end {
            let val = u16::from(data[i + 3] ^ data[i])
                .wrapping_add(u16::from(data[i + 1]) << 8);
            seed = seed.wrapping_add(val);
            i += 4;
        }

        seed
    }

    /// Appends a chunk to the signatures array if the chunk is large enough.
    ///
    /// Returns `true` on success, `false` if the signature table is full
    /// or the chunk size exceeds 65535.
    ///
    /// Ported from FreeRDP's `xcrush_append_chunk`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "size is bounded to <= 65535, fits in u16"
    )]
    fn append_chunk(&mut self, data: &[u8], beg: &mut usize, end: usize) -> bool {
        if self.signature_index >= self.signature_count {
            return false;
        }

        let size = end.saturating_sub(*beg);

        if size > 65535 {
            return false;
        }

        if size >= 15 {
            let seed = Self::update_hash(&data[*beg..], size);
            self.signatures[self.signature_index].size = size as u16;
            self.signatures[self.signature_index].seed = seed;
            self.signature_index += 1;
            *beg = end;
        }

        true
    }

    /// Computes chunk boundaries using a 32-byte rolling hash.
    ///
    /// Splits `data` into variable-sized chunks based on where the rolling
    /// hash accumulator satisfies `accumulator & 0x7F == 0`. Populates
    /// the `signatures` array with hash seeds and chunk sizes.
    ///
    /// Returns the number of signatures computed, or 0 if the input is
    /// too small (< 128 bytes) or an error occurs.
    ///
    /// Ported from FreeRDP's `xcrush_compute_chunks` + `xcrush_compute_signatures`.
    pub(crate) fn compute_signatures(&mut self, data: &[u8]) -> usize {
        self.signature_index = 0;

        let size = data.len();
        if size < 128 {
            return 0;
        }

        // Initialize the rolling hash with the first 32 bytes
        let mut accumulator: u32 = 0;
        for byte in &data[..32] {
            let rotation = accumulator.rotate_left(1);
            accumulator = u32::from(*byte) ^ rotation;
        }

        let mut offset: usize = 0; // start of current chunk
        let limit = size - 64;
        let mut i: usize = 0;

        // Process bytes in batches of 4 (matching FreeRDP's unrolled loop)
        while i < limit {
            for _ in 0..4 {
                let rotation = accumulator.rotate_left(1);
                accumulator = u32::from(data[i + 32]) ^ u32::from(data[i]) ^ rotation;

                if accumulator & 0x7F == 0
                    && !self.append_chunk(data, &mut offset, i + 32)
                {
                    return 0;
                }

                i += 1;
            }
        }

        // Append final chunk (remaining bytes)
        if offset < size && !self.append_chunk(data, &mut offset, size) {
            return 0;
        }

        self.signature_index
    }

    // ========================
    // Match finding and optimization (compression)
    // ========================

    /// Clears entries in the chunk hash table that fall within `[beg, end]`.
    ///
    /// Ported from FreeRDP's `xcrush_clear_hash_table_range`.
    fn clear_hash_table_range(&mut self, beg: u32, end: u32) {
        for entry in self.next_chunks.iter_mut() {
            let v = u32::from(*entry);
            if v >= beg && v <= end {
                *entry = 0;
            }
        }
        for chunk in self.chunks[..MAX_CHUNKS].iter_mut() {
            if chunk.next >= beg && chunk.next <= end {
                chunk.next = 0;
            }
        }
    }

    /// Finds the next chunk in the chain with a matching signature seed.
    ///
    /// Returns `Some(index)` of the next matching chunk, or `None` if
    /// there is no next chunk or the chain is invalid.
    ///
    /// Ported from FreeRDP's `xcrush_find_next_matching_chunk`.
    #[expect(
        clippy::as_conversions,
        reason = "u32 chunk indices safely converted to usize for array indexing"
    )]
    fn find_next_matching_chunk(&self, chunk_index: u32) -> Result<Option<u32>, BulkError> {
        if chunk_index as usize >= MAX_CHUNKS {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: chunk index out of range",
            ));
        }

        let chunk = &self.chunks[chunk_index as usize];
        if chunk.next == 0 {
            return Ok(None);
        }

        if chunk_index < self.chunk_head || chunk.next >= self.chunk_head {
            if chunk.next as usize >= MAX_CHUNKS {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH: next chunk index out of range",
                ));
            }
            return Ok(Some(chunk.next));
        }

        Ok(None)
    }

    /// Inserts a chunk into the hash table keyed by signature seed.
    ///
    /// Returns the index of a previously-existing chunk with the same seed
    /// (for match finding), or `None` if no previous chunk exists.
    ///
    /// Ported from FreeRDP's `xcrush_insert_chunk`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "chunk indices bounded to < 65534, fit in u16/u32"
    )]
    fn insert_chunk(
        &mut self,
        signature: &XCrushSignature,
        offset: u32,
    ) -> Result<Option<u32>, BulkError> {
        if self.chunk_head >= 65530 {
            self.chunk_head = 1;
            self.chunk_tail = 1;
        }

        if self.chunk_head >= self.chunk_tail {
            self.clear_hash_table_range(self.chunk_tail, self.chunk_tail + 10000);
            self.chunk_tail += 10000;
        }

        let index = self.chunk_head;
        self.chunk_head += 1;

        if self.chunk_head as usize >= MAX_CHUNKS {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: chunk head overflow",
            ));
        }

        self.chunks[index as usize].offset = offset;
        let seed = usize::from(signature.seed);
        let prev_chunk_index = if self.next_chunks[seed] != 0 {
            if usize::from(self.next_chunks[seed]) >= MAX_CHUNKS {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH: next_chunks index out of range",
                ));
            }
            Some(u32::from(self.next_chunks[seed]))
        } else {
            None
        };

        self.chunks[index as usize].next = u32::from(self.next_chunks[seed]);
        self.next_chunks[seed] = index as u16;
        Ok(prev_chunk_index)
    }

    /// Finds the match length between two positions in the history buffer.
    ///
    /// Searches both forward and backward from the match point to find
    /// the longest matching region.
    ///
    /// Returns the total match length (0 if < 11 bytes or a quick-reject
    /// heuristic fails), or an error for invalid offsets.
    ///
    /// Ported from FreeRDP's `xcrush_find_match_length`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "offsets and lengths bounded by history buffer size (2MB), safely fit in u32"
    )]
    fn find_match_length(
        &self,
        match_offset: usize,
        chunk_offset: usize,
        history_offset: usize,
        src_size: usize,
        max_match_length: usize,
    ) -> Result<Option<XCrushMatchInfo>, BulkError> {
        let history_buffer_size = self.history_buffer_size;
        let buf_end = history_offset + src_size;

        if match_offset > history_buffer_size {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: match_offset exceeds history buffer",
            ));
        }
        if chunk_offset > history_buffer_size {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: chunk_offset exceeds history buffer",
            ));
        }
        if match_offset == chunk_offset {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: match_offset == chunk_offset",
            ));
        }

        let buf = &*self.history_buffer;

        // Quick-reject heuristic: if byte at max_match_length+1 doesn't match, skip
        if match_offset + max_match_length + 1 < buf_end
            && buf[match_offset + max_match_length + 1]
                != buf[chunk_offset + max_match_length + 1]
        {
            return Ok(None);
        }

        // Forward matching
        let mut forward_len: usize = 0;
        let mut fm = match_offset;
        let mut fc = chunk_offset;
        loop {
            if buf[fm] != buf[fc] {
                break;
            }
            fm += 1;
            fc += 1;
            if fm > buf_end {
                break;
            }
            forward_len += 1;
        }

        // Reverse matching
        let mut reverse_len: usize = 0;
        if match_offset > 0 && chunk_offset > 0 {
            let mut rm = match_offset - 1;
            let mut rc = chunk_offset - 1;
            while rm > history_offset && rc > 0 && buf[rm] == buf[rc] {
                reverse_len += 1;
                if rm == 0 || rc == 0 {
                    break;
                }
                rm -= 1;
                rc -= 1;
            }
        }

        let total_len = reverse_len + forward_len;
        if total_len < 11 {
            return Ok(None);
        }

        let match_start = match_offset - reverse_len;
        let chunk_start = chunk_offset - reverse_len;

        Ok(Some(XCrushMatchInfo {
            match_offset: match_start as u32,
            chunk_offset: chunk_start as u32,
            match_length: total_len as u32,
        }))
    }

    /// Finds all matches between computed signatures and existing chunks
    /// in the hash table.
    ///
    /// Populates `original_matches` with the best match for each signature
    /// position. Returns the number of matches found.
    ///
    /// Ported from FreeRDP's `xcrush_find_all_matches`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "index/offset arithmetic bounded by history buffer size and u32 match fields"
    )]
    fn find_all_matches(
        &mut self,
        signature_index: usize,
        history_offset: usize,
        src_size: usize,
    ) -> Result<usize, BulkError> {
        let mut j: usize = 0;
        let mut src_offset: usize = 0;
        let mut prev_match_end: usize = 0;

        for i in 0..signature_index {
            let sig_size = self.signatures[i].size;
            if sig_size == 0 {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH: signature size is zero",
                ));
            }

            let offset = (src_offset + history_offset) as u32;

            // Make a copy of the signature for insert
            let sig_copy = self.signatures[i];
            let prev_chunk_idx = self.insert_chunk(&sig_copy, offset)?;

            if let Some(mut chunk_idx) = prev_chunk_idx {
                if src_offset + history_offset + sig_size as usize >= prev_match_end {
                    let mut max_match_length: usize = 0;
                    let mut best_match: Option<XCrushMatchInfo> = None;
                    let mut chunk_count: usize = 0;

                    loop {
                        let chunk_offset = self.chunks[chunk_idx as usize].offset as usize;

                        if chunk_offset < history_offset
                            || chunk_offset < offset as usize
                            || chunk_offset > src_size + history_offset
                        {
                            let result = self.find_match_length(
                                offset as usize,
                                chunk_offset,
                                history_offset,
                                src_size,
                                max_match_length,
                            )?;

                            if let Some(info) = result {
                                let match_len = info.match_length as usize;
                                if match_len > max_match_length {
                                    max_match_length = match_len;
                                    best_match = Some(info);
                                    if match_len > 256 {
                                        break;
                                    }
                                }
                            }
                        }

                        chunk_count += 1;
                        if chunk_count > 4 {
                            break;
                        }

                        match self.find_next_matching_chunk(chunk_idx)? {
                            Some(next) => chunk_idx = next,
                            None => break,
                        }
                    }

                    if let Some(best) = best_match {
                        self.original_matches[j] = best;

                        if (self.original_matches[j].match_offset as usize) < history_offset {
                            return Err(BulkError::InvalidCompressedData(
                                "XCRUSH: match offset before history",
                            ));
                        }

                        prev_match_end = self.original_matches[j].match_length as usize
                            + self.original_matches[j].match_offset as usize;
                        j += 1;

                        if j >= MAX_MATCH_COUNT {
                            return Err(BulkError::InvalidCompressedData(
                                "XCRUSH: too many matches",
                            ));
                        }
                    }
                }
            }

            src_offset += sig_size as usize;
            if src_offset > src_size {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH: src_offset exceeds src_size",
                ));
            }
        }

        if src_offset > src_size {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: final src_offset exceeds src_size",
            ));
        }

        Ok(j)
    }

    /// Optimizes matches by removing overlaps and adjusting boundaries.
    ///
    /// Takes the raw matches from `original_matches` and produces a
    /// non-overlapping set in `optimized_matches`.
    ///
    /// Returns the total match length across all optimized matches.
    ///
    /// Ported from FreeRDP's `xcrush_optimize_matches`.
    #[expect(
        clippy::as_conversions,
        reason = "u32 match fields compared and arithmetically combined"
    )]
    fn optimize_matches(&mut self) -> Result<usize, BulkError> {
        let mut j: usize = 0;
        let mut prev_match_end: u32 = 0;
        let mut total_match_length: usize = 0;
        let original_match_count = self.original_match_count;

        for i in 0..original_match_count {
            let orig = self.original_matches[i];

            if orig.match_offset <= prev_match_end {
                // Overlapping: only include if the extension is large enough
                if orig.match_offset < prev_match_end
                    && orig.match_length + orig.match_offset > prev_match_end + 6
                {
                    let match_diff = prev_match_end - orig.match_offset;

                    if orig.match_length <= match_diff {
                        return Err(BulkError::InvalidCompressedData(
                            "XCRUSH: optimized match length underflow",
                        ));
                    }
                    if match_diff >= 20000 {
                        return Err(BulkError::InvalidCompressedData(
                            "XCRUSH: match diff too large",
                        ));
                    }

                    self.optimized_matches[j] = XCrushMatchInfo {
                        match_offset: orig.match_offset + match_diff,
                        chunk_offset: orig.chunk_offset + match_diff,
                        match_length: orig.match_length - match_diff,
                    };

                    prev_match_end = self.optimized_matches[j].match_length
                        + self.optimized_matches[j].match_offset;
                    total_match_length += self.optimized_matches[j].match_length as usize;
                    j += 1;
                }
            } else {
                // Non-overlapping: include as-is
                self.optimized_matches[j] = orig;
                prev_match_end = orig.match_length + orig.match_offset;
                total_match_length += orig.match_length as usize;
                j += 1;
            }
        }

        self.optimized_match_count = j;
        Ok(total_match_length)
    }

    /// Generates the Level-1 compressed output format.
    ///
    /// Writes the match count, match details array, and literal data into
    /// the output buffer.
    ///
    /// Returns the total size of the compressed output.
    ///
    /// Ported from FreeRDP's `xcrush_generate_output`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "match fields bounded to u16/u32; output offsets bounded by buffer size"
    )]
    fn generate_output(
        &self,
        output_buffer: &mut [u8],
        history_offset: usize,
    ) -> Result<usize, BulkError> {
        let match_count = self.optimized_match_count;
        let output_size = output_buffer.len();

        if output_size < 2 {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: output buffer too small for header",
            ));
        }

        // Write match count (u16 LE)
        let count_bytes = (match_count as u16).to_le_bytes();
        output_buffer[0] = count_bytes[0];
        output_buffer[1] = count_bytes[1];

        // Match details start at offset 2, each entry is 8 bytes
        let match_details_end = 2 + match_count * 8;
        let mut literals_pos = match_details_end;

        if literals_pos > output_size {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: match details exceed output buffer",
            ));
        }

        // Write match detail entries
        for mi in 0..match_count {
            let d = 2 + mi * 8;
            let m = &self.optimized_matches[mi];

            let match_length = m.match_length as u16;
            let match_output_offset = (m.match_offset as usize)
                .checked_sub(history_offset)
                .ok_or(BulkError::InvalidCompressedData(
                    "XCRUSH: match offset before history",
                ))? as u16;
            let match_history_offset = m.chunk_offset;

            output_buffer[d..d + 2].copy_from_slice(&match_length.to_le_bytes());
            output_buffer[d + 2..d + 4].copy_from_slice(&match_output_offset.to_le_bytes());
            output_buffer[d + 4..d + 8].copy_from_slice(&match_history_offset.to_le_bytes());
        }

        // Write literal data (bytes between and after matches)
        let mut current_offset = history_offset;

        for mi in 0..match_count {
            let m = &self.optimized_matches[mi];
            let match_offset = m.match_offset as usize;
            let match_length = m.match_length as usize;

            if match_offset > current_offset {
                let literal_len = match_offset - current_offset;
                if literals_pos + literal_len >= output_size {
                    return Err(BulkError::InvalidCompressedData(
                        "XCRUSH: literal data exceeds output buffer",
                    ));
                }
                output_buffer[literals_pos..literals_pos + literal_len]
                    .copy_from_slice(&self.history_buffer[current_offset..match_offset]);
                literals_pos += literal_len;
                current_offset = match_offset + match_length;
            } else if match_offset == current_offset {
                current_offset = match_offset + match_length;
            } else {
                return Err(BulkError::InvalidCompressedData(
                    "XCRUSH: match offset before current position",
                ));
            }
        }

        // Copy trailing literals
        let trailing_len = self.history_offset - current_offset;
        if literals_pos + trailing_len >= output_size {
            return Err(BulkError::InvalidCompressedData(
                "XCRUSH: trailing literals exceed output buffer",
            ));
        }
        output_buffer[literals_pos..literals_pos + trailing_len]
            .copy_from_slice(&self.history_buffer[current_offset..self.history_offset]);
        literals_pos += trailing_len;

        Ok(literals_pos)
    }

    /// Resets the XCRUSH context.
    ///
    /// Zeros the signature, chunk, and match arrays.
    /// If `flush` is `true`, sets `history_offset` to `history_buffer_size + 1`
    /// (sentinel indicating a flush). Otherwise sets `history_offset` to 0.
    /// Also resets the inner MPPC context.
    pub(crate) fn reset(&mut self, flush: bool) {
        self.signature_index = 0;
        self.signature_count = MAX_SIGNATURE_COUNT;
        for sig in self.signatures.iter_mut() {
            *sig = XCrushSignature::default();
        }
        self.compression_flags = 0;
        self.chunk_head = 1;
        self.chunk_tail = 1;
        for chunk in self.chunks.iter_mut() {
            *chunk = XCrushChunk::default();
        }
        self.next_chunks.fill(0);
        for m in self.original_matches.iter_mut() {
            *m = XCrushMatchInfo::default();
        }
        for m in self.optimized_matches.iter_mut() {
            *m = XCrushMatchInfo::default();
        }
        self.original_match_count = 0;
        self.optimized_match_count = 0;

        if flush {
            self.history_offset = self.history_buffer_size + 1;
        } else {
            self.history_offset = 0;
        }

        self.mppc.reset(flush);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xcrush_context_new_decompressor() {
        let ctx = XCrushContext::new(false);
        assert_eq!(ctx.history_buffer_size, HISTORY_BUFFER_SIZE);
        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.signature_index, 0);
        assert_eq!(ctx.signature_count, MAX_SIGNATURE_COUNT);
        assert_eq!(ctx.chunk_head, 1);
        assert_eq!(ctx.chunk_tail, 1);
        assert_eq!(ctx.compression_flags, 0);
        assert_eq!(ctx.original_match_count, 0);
        assert_eq!(ctx.optimized_match_count, 0);
    }

    #[test]
    fn test_xcrush_context_new_compressor() {
        let ctx = XCrushContext::new(true);
        assert_eq!(ctx.history_buffer_size, HISTORY_BUFFER_SIZE);
        assert_eq!(ctx.history_offset, 0);
    }

    #[test]
    fn test_xcrush_context_reset_no_flush() {
        let mut ctx = XCrushContext::new(false);
        ctx.history_offset = 12345;
        ctx.signature_index = 42;
        ctx.chunk_head = 100;
        ctx.chunk_tail = 200;
        ctx.compression_flags = 0xFF;
        ctx.original_match_count = 5;
        ctx.optimized_match_count = 3;

        ctx.reset(false);

        assert_eq!(ctx.history_offset, 0);
        assert_eq!(ctx.signature_index, 0);
        assert_eq!(ctx.signature_count, MAX_SIGNATURE_COUNT);
        assert_eq!(ctx.chunk_head, 1);
        assert_eq!(ctx.chunk_tail, 1);
        assert_eq!(ctx.compression_flags, 0);
        assert_eq!(ctx.original_match_count, 0);
        assert_eq!(ctx.optimized_match_count, 0);
    }

    #[test]
    fn test_xcrush_context_reset_flush() {
        let mut ctx = XCrushContext::new(false);
        ctx.reset(true);

        assert_eq!(ctx.history_offset, HISTORY_BUFFER_SIZE + 1);
        assert_eq!(ctx.signature_index, 0);
        assert_eq!(ctx.chunk_head, 1);
        assert_eq!(ctx.chunk_tail, 1);
    }

    // ========================
    // L1 decompression tests
    // ========================

    #[test]
    fn test_decompress_l1_no_compression() {
        let mut ctx = XCrushContext::new(false);
        let data = b"hello, world!";
        let result = ctx
            .decompress_l1(data, flags::L1_NO_COMPRESSION | flags::L1_PACKET_AT_FRONT)
            .unwrap();
        assert_eq!(result, b"hello, world!");
        assert_eq!(ctx.history_offset, 13);
    }

    #[test]
    fn test_decompress_l1_compressed_no_matches() {
        let mut ctx = XCrushContext::new(false);
        // Build a compressed packet with 0 matches: just literals
        // Format: [match_count: u16 LE] [match_details...] [literals...]
        let mut packet = Vec::new();
        packet.extend_from_slice(&0u16.to_le_bytes()); // 0 matches
        packet.extend_from_slice(b"test data"); // all literals
        let result = ctx
            .decompress_l1(&packet, flags::L1_COMPRESSED | flags::L1_PACKET_AT_FRONT)
            .unwrap();
        assert_eq!(result, b"test data");
    }

    #[test]
    fn test_decompress_l1_compressed_with_match() {
        let mut ctx = XCrushContext::new(false);

        // Pre-populate history buffer with "ABCDEFGH" at offset 0
        ctx.history_buffer[..8].copy_from_slice(b"ABCDEFGH");
        ctx.history_offset = 8;

        // Build a compressed packet:
        // - 1 match: length=4, output_offset=5, history_offset=2 (copies "CDEF" from history)
        // - Literals: "Hello" (placed at output offset 0-4)
        let mut packet = Vec::new();
        packet.extend_from_slice(&1u16.to_le_bytes()); // 1 match
        // Match detail: MatchLength=4, MatchOutputOffset=5, MatchHistoryOffset=2
        packet.extend_from_slice(&4u16.to_le_bytes());
        packet.extend_from_slice(&5u16.to_le_bytes());
        packet.extend_from_slice(&2u32.to_le_bytes());
        // Literals: "Hello" (5 bytes before the match)
        packet.extend_from_slice(b"Hello");

        let result = ctx
            .decompress_l1(&packet, flags::L1_COMPRESSED)
            .unwrap();

        // Expected output: "Hello" + "CDEF" = "HelloCDEF"
        assert_eq!(result, b"HelloCDEF");
    }

    #[test]
    fn test_decompress_l1_empty_input_error() {
        let mut ctx = XCrushContext::new(false);
        let result = ctx.decompress_l1(&[], flags::L1_COMPRESSED);
        assert!(result.is_err());
    }

    #[test]
    fn test_decompress_l1_invalid_flags_error() {
        let mut ctx = XCrushContext::new(false);
        // Neither L1_NO_COMPRESSION nor L1_COMPRESSED set
        let result = ctx.decompress_l1(b"data", 0);
        assert!(result.is_err());
    }

    // ========================
    // Full decompress tests
    // ========================

    #[test]
    fn test_decompress_no_l2_no_l1_compression() {
        let mut ctx = XCrushContext::new(false);
        // Header: [L1_flags, L2_flags] + data
        // L1_NO_COMPRESSION(0x02) | L1_PACKET_AT_FRONT(0x04) = 0x06
        let mut packet = vec![0x06u8, 0x00u8];
        packet.extend_from_slice(b"raw data here");

        let result = ctx.decompress(&packet, 0).unwrap();
        assert_eq!(result, b"raw data here");
    }

    #[test]
    fn test_decompress_too_short_error() {
        let mut ctx = XCrushContext::new(false);
        let result = ctx.decompress(&[0x00], 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_decompress_flushed_clears_history() {
        let mut ctx = XCrushContext::new(false);
        // Write some data to history
        ctx.history_buffer[0] = 0xFF;
        ctx.history_offset = 100;

        // L1_NO_COMPRESSION(0x02) | L1_PACKET_AT_FRONT(0x04) = 0x06
        let mut packet = vec![0x06u8, 0x00u8];
        packet.extend_from_slice(b"test");

        let result = ctx.decompress(&packet, flags::PACKET_FLUSHED).unwrap();
        assert_eq!(result, b"test");
        // History should have been cleared
        assert_eq!(ctx.history_buffer[0], b't'); // first byte of "test"
    }

    // ========================
    // Chunk computation tests
    // ========================

    #[test]
    fn test_update_hash_small() {
        // Deterministic: same input should give same hash
        let data = [0x41u8, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48];
        let h1 = XCrushContext::update_hash(&data, 8);
        let h2 = XCrushContext::update_hash(&data, 8);
        assert_eq!(h1, h2);
        // Seed 5381 for size <= 32
        assert_ne!(h1, 5381); // hash should have changed from seed
    }

    #[test]
    fn test_update_hash_large_uses_different_seed() {
        let data = [0xAA; 64];
        let h_small = XCrushContext::update_hash(&data, 32);
        let h_large = XCrushContext::update_hash(&data, 33); // > 32: seed 5413, only hashes first 32
        // Different seeds should (very likely) produce different results
        assert_ne!(h_small, h_large);
    }

    #[test]
    fn test_compute_signatures_small_input() {
        let mut ctx = XCrushContext::new(true);
        // Input < 128 bytes: should return 0
        let data = [0u8; 100];
        let count = ctx.compute_signatures(&data);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_compute_signatures_128_bytes() {
        let mut ctx = XCrushContext::new(true);
        // Exactly 128 bytes: should produce at least 1 signature (the final chunk)
        let mut data = [0u8; 128];
        // Fill with some non-zero data to exercise the hash
        for (i, b) in data.iter_mut().enumerate() {
            *b = u8::try_from(i & 0xFF).unwrap();
        }
        let count = ctx.compute_signatures(&data);
        // Should have at least 1 signature (the trailing chunk)
        assert!(count >= 1, "expected at least 1 signature, got {count}");
    }

    #[test]
    fn test_compute_signatures_large_input() {
        let mut ctx = XCrushContext::new(true);
        // ~1 KB of sequential data
        let mut data = [0u8; 1024];
        for (i, b) in data.iter_mut().enumerate() {
            *b = u8::try_from(i.wrapping_mul(17) & 0xFF).unwrap();
        }
        let count = ctx.compute_signatures(&data);
        // Should have some signatures (depends on rolling hash behavior)
        assert!(count >= 1, "expected at least 1 signature, got {count}");
        // All signatures should have non-zero size
        for sig in &ctx.signatures[..count] {
            assert!(sig.size > 0, "signature size should be > 0");
        }
    }

    #[test]
    fn test_compute_signatures_deterministic() {
        let mut ctx1 = XCrushContext::new(true);
        let mut ctx2 = XCrushContext::new(true);
        let data = b"The quick brown fox jumps over the lazy dog repeatedly and repeatedly and repeatedly until we get enough data to reach the minimum threshold for xcrush chunk computation which is 128 bytes of input data.";
        let count1 = ctx1.compute_signatures(data);
        let count2 = ctx2.compute_signatures(data);
        assert_eq!(count1, count2);
        for i in 0..count1 {
            assert_eq!(ctx1.signatures[i].seed, ctx2.signatures[i].seed);
            assert_eq!(ctx1.signatures[i].size, ctx2.signatures[i].size);
        }
    }
}
