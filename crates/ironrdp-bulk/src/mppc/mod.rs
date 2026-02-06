//! MPPC (Microsoft Point-to-Point Compression) implementation.
//!
//! Supports RDP4 (8K history) and RDP5 (64K history) compression levels.

pub(crate) mod tables;

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

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
}
