#![doc = "Bulk compression algorithms (MPPC, XCRUSH, NCRUSH) for IronRDP."]
//!
//! This crate implements the RDP bulk compression algorithms, ported from
//! [FreeRDP](https://github.com/FreeRDP/FreeRDP). It supports compression and
//! decompression for all four RDP compression levels:
//!
//! | Level | Algorithm | History Buffer | RDP Version |
//! |-------|-----------|---------------|-------------|
//! | `Rdp4`  | MPPC      | 8 KB          | RDP 4.0     |
//! | `Rdp5`  | MPPC      | 64 KB         | RDP 5.0     |
//! | `Rdp6`  | NCRUSH    | 64 KB         | RDP 6.0     |
//! | `Rdp61` | XCRUSH    | 2 MB          | RDP 6.1     |
//!
//! # Usage
//!
//! ```rust
//! use ironrdp_bulk::{BulkCompressor, CompressionType, flags};
//!
//! // Create sender (compressor) and receiver (decompressor)
//! let mut sender = BulkCompressor::new(CompressionType::Rdp5).unwrap();
//! let mut receiver = BulkCompressor::new(CompressionType::Rdp5).unwrap();
//!
//! let input = b"Hello world! Hello world! Hello world! Hello world! x";
//!
//! // Compress
//! let (compressed_size, compress_flags) = sender.compress(input).unwrap();
//!
//! if compress_flags & flags::PACKET_COMPRESSED != 0 {
//!     // Compressed data is available
//!     let compressed = sender.compressed_data(compressed_size);
//!     assert!(compressed.len() < input.len());
//!
//!     // Decompress
//!     let decompressed = receiver.decompress(compressed, compress_flags).unwrap();
//!     assert_eq!(decompressed, input);
//! }
//! ```
//!
//! # Features
//!
//! - **`std`** (default): Enables standard library support.
//! - **`alloc`** (implied by `std`): Enables heap allocation without `std`,
//!   suitable for `no_std` environments such as WebAssembly.
//!
//! # Safety
//!
//! This crate contains **zero `unsafe` code**. The `#![forbid(unsafe_code)]`
//! attribute enforces this invariant at compile time. All numeric casts are
//! documented with `#[expect]` attributes explaining their safety bounds.
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]
#![cfg_attr(doc, warn(missing_docs))]

#[cfg(feature = "alloc")]
extern crate alloc;

mod bitstream;
mod bulk;
mod error;
mod mppc;
mod ncrush;
mod xcrush;

pub use self::bulk::BulkCompressor;
pub use self::error::BulkError;

/// RDP bulk compression type (low 4 bits of compression flags).
///
/// Determines which compression algorithm to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionType {
    /// MPPC with 8K history buffer (RDP 4.0)
    Rdp4 = 0x00,
    /// MPPC with 64K history buffer (RDP 5.0)
    Rdp5 = 0x01,
    /// NCRUSH Huffman-based compression (RDP 6.0)
    Rdp6 = 0x02,
    /// XCRUSH two-level compression (RDP 6.1)
    Rdp61 = 0x03,
}

impl CompressionType {
    /// Attempts to parse a compression type from the low 4 bits of a flags byte.
    pub fn from_flags(flags: u32) -> Result<Self, BulkError> {
        match flags & flags::COMPRESSION_TYPE_MASK {
            0x00 => Ok(Self::Rdp4),
            0x01 => Ok(Self::Rdp5),
            0x02 => Ok(Self::Rdp6),
            0x03 => Ok(Self::Rdp61),
            other => Err(BulkError::UnsupportedCompressionType(other)),
        }
    }
}

impl core::fmt::Display for CompressionType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rdp4 => write!(f, "RDP4 (MPPC 8K)"),
            Self::Rdp5 => write!(f, "RDP5 (MPPC 64K)"),
            Self::Rdp6 => write!(f, "RDP6 (NCRUSH)"),
            Self::Rdp61 => write!(f, "RDP6.1 (XCRUSH)"),
        }
    }
}

/// Level-2 and Level-1 compression flag constants.
///
/// These correspond to the flags defined in FreeRDP's `bulk.h`.
pub mod flags {
    /// Level-2 flag: data is compressed.
    pub const PACKET_COMPRESSED: u32 = 0x20;
    /// Level-2 flag: history buffer reset to beginning.
    pub const PACKET_AT_FRONT: u32 = 0x40;
    /// Level-2 flag: history buffer was flushed (reset).
    pub const PACKET_FLUSHED: u32 = 0x80;
    /// Mask to extract the compression type from the flags byte.
    pub const COMPRESSION_TYPE_MASK: u32 = 0x0F;

    /// Level-1 flag (XCRUSH): history buffer reset to front.
    pub const L1_PACKET_AT_FRONT: u32 = 0x04;
    /// Level-1 flag (XCRUSH): data is not compressed at Level-1.
    pub const L1_NO_COMPRESSION: u32 = 0x02;
    /// Level-1 flag (XCRUSH): data is compressed at Level-1.
    pub const L1_COMPRESSED: u32 = 0x01;
    /// Level-1 flag (XCRUSH): inner (Level-2/MPPC) compression was applied.
    pub const L1_INNER_COMPRESSION: u32 = 0x10;
}
