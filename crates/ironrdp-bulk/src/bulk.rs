//! Bulk compressor coordinator that routes to the appropriate algorithm.
//!
//! Holds send/receive context pairs for MPPC, NCRUSH, and XCRUSH.
//! Selects the appropriate compressor based on the configured compression
//! level (for compression) or the type bits in the flags (for decompression).
//!
//! Ported from FreeRDP's `libfreerdp/codec/bulk.c`.

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use crate::error::BulkError;
use crate::mppc::MppcContext;
use crate::ncrush::NCrushContext;
use crate::xcrush::XCrushContext;
use crate::CompressionType;

/// Size of the internal output buffer used for compression.
const OUTPUT_BUFFER_SIZE: usize = 65536;

/// Minimum input size for compression (below this, data is sent uncompressed).
const COMPRESS_MIN_SIZE: usize = 50;

/// Maximum input size for compression (above this, data is sent uncompressed).
const COMPRESS_MAX_SIZE: usize = 16384;

/// Bulk compression/decompression coordinator.
///
/// Manages send (compression) and receive (decompression) context pairs
/// for all three RDP compression algorithms. Routes compress/decompress
/// requests to the appropriate algorithm based on the compression level
/// or type flags.
///
/// Ported from FreeRDP's `rdp_bulk` struct.
pub struct BulkCompressor {
    /// The compression level to use for outgoing data.
    compression_level: CompressionType,
    /// MPPC context for sending (compression).
    mppc_send: MppcContext,
    /// MPPC context for receiving (decompression).
    mppc_recv: MppcContext,
    /// NCRUSH context for sending (compression).
    ncrush_send: NCrushContext,
    /// NCRUSH context for receiving (decompression).
    ncrush_recv: NCrushContext,
    /// XCRUSH context for sending (compression).
    xcrush_send: XCrushContext,
    /// XCRUSH context for receiving (decompression).
    xcrush_recv: XCrushContext,
    /// Internal output buffer for compressed data.
    output_buffer: Box<[u8; OUTPUT_BUFFER_SIZE]>,
}

impl BulkCompressor {
    /// Creates a new bulk compressor/decompressor with the given compression level.
    ///
    /// Allocates send and receive contexts for MPPC, NCRUSH, and XCRUSH.
    /// The `compression_level` determines which algorithm is used for
    /// outbound compression. Inbound decompression automatically selects
    /// the algorithm based on the type bits in the packet flags.
    ///
    /// # Compression Levels
    ///
    /// - `Rdp4` (0x00): MPPC with 8K history buffer
    /// - `Rdp5` (0x01): MPPC with 64K history buffer
    /// - `Rdp6` (0x02): NCRUSH (Huffman-based)
    /// - `Rdp61` (0x03): XCRUSH (two-level: chunk matching + MPPC)
    ///
    /// Ported from FreeRDP's `bulk_new`.
    pub fn new(compression_level: CompressionType) -> Result<Self, BulkError> {
        // FreeRDP creates MPPC with level 1 by default; the level is
        // adjusted dynamically per-call in compress/decompress.
        let mppc_send = MppcContext::new(1, true);
        let mppc_recv = MppcContext::new(1, false);
        let ncrush_send = NCrushContext::new(true)?;
        let ncrush_recv = NCrushContext::new(false)?;
        let xcrush_send = XCrushContext::new(true);
        let xcrush_recv = XCrushContext::new(false);

        // Heap-allocate the 64KB output buffer to avoid stack overflow
        let output_buffer = {
            let v: alloc::vec::Vec<u8> = alloc::vec![0u8; OUTPUT_BUFFER_SIZE];
            let boxed_slice = v.into_boxed_slice();
            // SAFETY: Vec was created with exactly OUTPUT_BUFFER_SIZE elements
            boxed_slice
                .try_into()
                .unwrap_or_else(|_| unreachable!())
        };

        Ok(Self {
            compression_level,
            mppc_send,
            mppc_recv,
            ncrush_send,
            ncrush_recv,
            xcrush_send,
            xcrush_recv,
            output_buffer,
        })
    }

    /// Returns the configured compression level.
    pub fn compression_level(&self) -> CompressionType {
        self.compression_level
    }

    /// Returns `true` if the input size is outside the compressible range.
    ///
    /// FreeRDP skips compression for sizes <= 50 or >= 16384.
    pub fn should_skip_compression(src_size: usize) -> bool {
        src_size <= COMPRESS_MIN_SIZE || src_size >= COMPRESS_MAX_SIZE
    }

    /// Returns a mutable reference to the output buffer for compression.
    pub(crate) fn output_buffer_mut(&mut self) -> &mut [u8; OUTPUT_BUFFER_SIZE] {
        &mut self.output_buffer
    }

    /// Returns a reference to the MPPC send context.
    pub(crate) fn mppc_send(&mut self) -> &mut MppcContext {
        &mut self.mppc_send
    }

    /// Returns a reference to the MPPC receive context.
    pub(crate) fn mppc_recv(&mut self) -> &mut MppcContext {
        &mut self.mppc_recv
    }

    /// Returns a reference to the NCRUSH send context.
    pub(crate) fn ncrush_send(&mut self) -> &mut NCrushContext {
        &mut self.ncrush_send
    }

    /// Returns a reference to the NCRUSH receive context.
    pub(crate) fn ncrush_recv(&mut self) -> &mut NCrushContext {
        &mut self.ncrush_recv
    }

    /// Returns a reference to the XCRUSH send context.
    pub(crate) fn xcrush_send(&mut self) -> &mut XCrushContext {
        &mut self.xcrush_send
    }

    /// Returns a reference to the XCRUSH receive context.
    pub(crate) fn xcrush_recv(&mut self) -> &mut XCrushContext {
        &mut self.xcrush_recv
    }

    /// Resets all compression and decompression contexts.
    ///
    /// Ported from FreeRDP's `bulk_reset`.
    pub fn reset(&mut self) {
        self.mppc_send.reset(false);
        self.mppc_recv.reset(false);
        self.ncrush_send.reset(false);
        self.ncrush_recv.reset(false);
        self.xcrush_send.reset(false);
        self.xcrush_recv.reset(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bulk_compressor_new_rdp4() {
        let bulk = BulkCompressor::new(CompressionType::Rdp4).unwrap();
        assert_eq!(bulk.compression_level(), CompressionType::Rdp4);
    }

    #[test]
    fn test_bulk_compressor_new_rdp5() {
        let bulk = BulkCompressor::new(CompressionType::Rdp5).unwrap();
        assert_eq!(bulk.compression_level(), CompressionType::Rdp5);
    }

    #[test]
    fn test_bulk_compressor_new_rdp6() {
        let bulk = BulkCompressor::new(CompressionType::Rdp6).unwrap();
        assert_eq!(bulk.compression_level(), CompressionType::Rdp6);
    }

    #[test]
    fn test_bulk_compressor_new_rdp61() {
        let bulk = BulkCompressor::new(CompressionType::Rdp61).unwrap();
        assert_eq!(bulk.compression_level(), CompressionType::Rdp61);
    }

    #[test]
    fn test_bulk_compressor_skip_small() {
        assert!(BulkCompressor::should_skip_compression(10));
        assert!(BulkCompressor::should_skip_compression(50));
    }

    #[test]
    fn test_bulk_compressor_skip_large() {
        assert!(BulkCompressor::should_skip_compression(16384));
        assert!(BulkCompressor::should_skip_compression(65536));
    }

    #[test]
    fn test_bulk_compressor_no_skip_normal() {
        assert!(!BulkCompressor::should_skip_compression(51));
        assert!(!BulkCompressor::should_skip_compression(8192));
        assert!(!BulkCompressor::should_skip_compression(16383));
    }

    #[test]
    fn test_bulk_compressor_reset() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp61).unwrap();
        // Should not panic
        bulk.reset();
    }

    #[test]
    fn test_bulk_compressor_contexts_independent() {
        let bulk = BulkCompressor::new(CompressionType::Rdp6).unwrap();
        // Send and receive NCRUSH contexts should be separate instances
        // (we can only verify they exist and the struct was created)
        assert_eq!(bulk.compression_level(), CompressionType::Rdp6);
    }
}
