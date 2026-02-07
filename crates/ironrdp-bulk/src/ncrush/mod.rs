//! NCRUSH (RDP 6.0) Huffman-based compression implementation.
//!
//! Uses Huffman coding with an LRU offset cache for LZ77-style
//! back-references. Operates on a 64 KB sliding-window history buffer.
//!
//! Ported from FreeRDP's `libfreerdp/codec/ncrush.c`.

pub(crate) mod tables;

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
