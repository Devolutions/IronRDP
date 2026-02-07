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

/// Mask for the compression control flags (COMPRESSED | AT_FRONT | FLUSHED).
/// Corresponds to FreeRDP's `BULK_COMPRESSION_FLAGS_MASK`.
const BULK_COMPRESSION_FLAGS_MASK: u32 = 0xE0;

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

    /// Decompresses bulk-compressed RDP data.
    ///
    /// `flags` contains the compression type (low 4 bits) and control flags
    /// (`PACKET_COMPRESSED`, `PACKET_AT_FRONT`, `PACKET_FLUSHED`).
    ///
    /// If no compression flags are set, returns `src_data` unchanged.
    /// Otherwise, routes to the appropriate algorithm based on the type bits:
    /// - `0x00` (RDP4): MPPC with 8K buffer
    /// - `0x01` (RDP5): MPPC with 64K buffer
    /// - `0x02` (RDP6): NCRUSH
    /// - `0x03` (RDP6.1): XCRUSH
    ///
    /// Ported from FreeRDP's `bulk_decompress`.
    pub fn decompress<'a>(
        &'a mut self,
        src_data: &'a [u8],
        flags: u32,
    ) -> Result<&'a [u8], BulkError> {
        let compression_flags = flags & BULK_COMPRESSION_FLAGS_MASK;

        // If no compression flags are set, return source data unchanged
        if compression_flags == 0 {
            return Ok(src_data);
        }

        let comp_type = CompressionType::from_flags(flags)?;

        match comp_type {
            CompressionType::Rdp4 => {
                self.mppc_recv.set_compression_level(0);
                self.mppc_recv.decompress(src_data, flags)
            }
            CompressionType::Rdp5 => {
                self.mppc_recv.set_compression_level(1);
                self.mppc_recv.decompress(src_data, flags)
            }
            CompressionType::Rdp6 => {
                self.ncrush_recv.decompress(src_data, flags)
            }
            CompressionType::Rdp61 => {
                self.xcrush_recv.decompress(src_data, flags)
            }
        }
    }

    /// Compresses data using the configured compression algorithm.
    ///
    /// Returns `Ok((compressed_size, flags))` on success:
    /// - If `flags & PACKET_COMPRESSED != 0`: compressed data is available
    ///   in the internal output buffer via [`compressed_data`].
    /// - If compression was skipped (size out of range) or the algorithm
    ///   flushed (compressed output larger than input): `flags` will **not**
    ///   have `PACKET_COMPRESSED` set, and the caller should transmit the
    ///   original `src_data` uncompressed.
    ///
    /// FreeRDP skips compression for sizes ≤ 50 or ≥ 16384.
    ///
    /// Ported from FreeRDP's `bulk_compress`.
    pub fn compress(
        &mut self,
        src_data: &[u8],
    ) -> Result<(usize, u32), BulkError> {
        let src_size = src_data.len();

        // Skip compression for edge case sizes
        if Self::should_skip_compression(src_size) {
            return Ok((src_size, 0));
        }

        match self.compression_level {
            CompressionType::Rdp4 => {
                self.mppc_send.set_compression_level(0);
                self.mppc_send.compress(src_data, &mut *self.output_buffer)
            }
            CompressionType::Rdp5 => {
                self.mppc_send.set_compression_level(1);
                self.mppc_send.compress(src_data, &mut *self.output_buffer)
            }
            CompressionType::Rdp6 => {
                self.ncrush_send.compress(src_data, &mut *self.output_buffer)
            }
            CompressionType::Rdp61 => {
                self.xcrush_send.compress(src_data, &mut *self.output_buffer)
            }
        }
    }

    /// Returns a slice of the internal output buffer containing compressed
    /// data from the most recent [`compress`] call.
    ///
    /// `size` should be the `compressed_size` value returned by `compress`.
    pub fn compressed_data(&self, size: usize) -> &[u8] {
        &self.output_buffer[..size]
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

    // ---------------------------------------------------------------
    // Compression skip tests
    // ---------------------------------------------------------------

    #[test]
    fn test_bulk_compress_skip_small_input() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp5).unwrap();
        let data = b"tiny"; // 4 bytes, below threshold
        let (size, flags) = bulk.compress(data).unwrap();
        assert_eq!(size, data.len());
        assert_eq!(flags, 0); // no compression applied
    }

    #[test]
    fn test_bulk_compress_skip_empty() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp5).unwrap();
        let data = b"";
        let (size, flags) = bulk.compress(data).unwrap();
        assert_eq!(size, 0);
        assert_eq!(flags, 0);
    }

    // ---------------------------------------------------------------
    // Decompress: no flags → pass-through
    // ---------------------------------------------------------------

    #[test]
    fn test_bulk_decompress_no_flags() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp5).unwrap();
        let data = b"uncompressed data";
        let result = bulk.decompress(data, 0x00).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_bulk_decompress_unsupported_type() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp5).unwrap();
        // flags = PACKET_COMPRESSED | type 0x0F (invalid)
        let result = bulk.decompress(b"data", 0x2F);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // Round-trip tests through the bulk API for each algorithm
    // ---------------------------------------------------------------

    /// Helper: compress with one BulkCompressor (sender) and decompress
    /// with another (receiver). Returns the decompressed data as a Vec.
    fn bulk_roundtrip(
        compression_level: CompressionType,
        input: &[u8],
    ) -> Vec<u8> {
        let mut sender = BulkCompressor::new(compression_level).unwrap();
        let mut receiver = BulkCompressor::new(compression_level).unwrap();

        let (comp_size, flags) = sender.compress(input).unwrap();

        if flags & crate::flags::PACKET_COMPRESSED != 0 {
            // Compressed: pass compressed data to receiver
            let compressed = sender.compressed_data(comp_size).to_vec();
            let decompressed = receiver.decompress(&compressed, flags).unwrap();
            decompressed.to_vec()
        } else {
            // Not compressed: data should be sent as-is
            input.to_vec()
        }
    }

    #[test]
    fn test_bulk_roundtrip_rdp5_mppc() {
        let input = b"The quick brown fox jumps over the lazy dog. \
                      The quick brown fox jumps over the lazy dog again.";
        let output = bulk_roundtrip(CompressionType::Rdp5, input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_bulk_roundtrip_rdp4_mppc() {
        let input = b"Hello world! Hello world! Hello world! Hello world! x";
        let output = bulk_roundtrip(CompressionType::Rdp4, input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_bulk_roundtrip_rdp6_ncrush() {
        let input = b"for.whom.the.bell.tolls,.the.bell.tolls.for.thee!xx";
        let output = bulk_roundtrip(CompressionType::Rdp6, input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_bulk_roundtrip_rdp61_xcrush() {
        let input = b"XCRUSH test data with repeated XCRUSH patterns for compression!!";
        let output = bulk_roundtrip(CompressionType::Rdp61, input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_bulk_roundtrip_rdp5_binary_data() {
        // Binary data with all byte values
        let mut input = Vec::new();
        for _ in 0..2 {
            for b in 0u8..=255 {
                input.push(b);
            }
        }
        // 512 bytes — within compressible range
        let output = bulk_roundtrip(CompressionType::Rdp5, &input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_bulk_roundtrip_rdp6_longer_text() {
        let input = b"The RDP protocol uses bulk compression to reduce bandwidth. \
                      Multiple algorithms are supported: MPPC for RDP4/5, \
                      NCRUSH for RDP6, and XCRUSH for RDP6.1. Each has \
                      different tradeoffs between speed and compression ratio.";
        let output = bulk_roundtrip(CompressionType::Rdp6, input);
        assert_eq!(output, input);
    }

    // ---------------------------------------------------------------
    // Routing verification
    // ---------------------------------------------------------------

    #[test]
    fn test_bulk_compress_rdp5_sets_type_bits() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp5).unwrap();
        let input = b"Some data that should compress with MPPC level 1 algorithm!!";
        let (_size, flags) = bulk.compress(input).unwrap();

        if flags & crate::flags::PACKET_COMPRESSED != 0 {
            // Type bits should be 0x01 (RDP5)
            let comp_type = flags & crate::flags::COMPRESSION_TYPE_MASK;
            assert_eq!(comp_type, 0x01, "Expected RDP5 type bits");
        }
    }

    #[test]
    fn test_bulk_compress_rdp6_sets_type_bits() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp6).unwrap();
        let input = b"for.whom.the.bell.tolls,.the.bell.tolls.for.thee!xx";
        let (_size, flags) = bulk.compress(input).unwrap();

        if flags & crate::flags::PACKET_COMPRESSED != 0 {
            // Type bits should be 0x02 (RDP6/NCRUSH)
            let comp_type = flags & crate::flags::COMPRESSION_TYPE_MASK;
            assert_eq!(comp_type, 0x02, "Expected RDP6 (NCRUSH) type bits");
        }
    }

    #[test]
    fn test_bulk_compress_rdp61_sets_type_bits() {
        let mut bulk = BulkCompressor::new(CompressionType::Rdp61).unwrap();
        let input = b"XCRUSH test data with repeated XCRUSH patterns for compression!!";
        let (_size, flags) = bulk.compress(input).unwrap();

        if flags & crate::flags::PACKET_COMPRESSED != 0 {
            // Type bits should be 0x03 (RDP6.1/XCRUSH)
            let comp_type = flags & crate::flags::COMPRESSION_TYPE_MASK;
            assert_eq!(comp_type, 0x03, "Expected RDP6.1 (XCRUSH) type bits");
        }
    }
}
