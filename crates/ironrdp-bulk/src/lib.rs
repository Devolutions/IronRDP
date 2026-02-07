#![doc = "Bulk compression algorithms (MPPC, XCRUSH, NCRUSH) for IronRDP"]
//!
//! # Safety
//!
//! This crate contains **zero `unsafe` code**. The `#![forbid(unsafe_code)]` attribute
//! is set to enforce this invariant at compile time.
//!
//! ## Audit Summary (TASK-033)
//!
//! | Category                   | Count | Notes                                                  |
//! |----------------------------|-------|--------------------------------------------------------|
//! | `unsafe` blocks            | 0     | None in the entire crate                               |
//! | Raw pointer usage          | 0     | No `*const`, `*mut`, `as_ptr()`, `as_mut_ptr()`        |
//! | `transmute` / `unchecked`  | 0     | No `mem::transmute`, `get_unchecked`, etc.              |
//! | `#[allow(unsafe_*)]`       | 0     | No safety lint suppression                             |
//! | `as` numeric casts         | ~161  | Porting artifact from C; tracked for TASK-034          |
//! | `wrapping_*` arithmetic    | 15    | All intentional for modular hash/offset computations   |
//! | `unwrap_or_else(unreachable)` | 2  | Infallible `Vec→Box<[T;N]>` conversions               |
//!
//! The `as` casts (mostly `as usize` for indexing and `as u32`/`u8`/`u16` for
//! bitstream operations) are the main area flagged for cleanup in TASK-034,
//! where they can be replaced with `.into()`, `From`/`TryFrom`, or explicit
//! truncation helpers as appropriate.
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
