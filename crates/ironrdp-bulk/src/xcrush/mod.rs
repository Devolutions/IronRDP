//! XCRUSH (RDP 6.1) two-level compression implementation.
//!
//! Level 1 uses chunk-based matching; Level 2 uses MPPC.
//!
//! Ported from FreeRDP's `libfreerdp/codec/xcrush.c`.

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec};

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
        // Modify some state
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

        // Flush sets history_offset to buffer_size + 1 (sentinel)
        assert_eq!(ctx.history_offset, HISTORY_BUFFER_SIZE + 1);
        assert_eq!(ctx.signature_index, 0);
        assert_eq!(ctx.chunk_head, 1);
        assert_eq!(ctx.chunk_tail, 1);
    }
}
