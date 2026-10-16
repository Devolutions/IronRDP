//! High-level ZGFX compression API for EGFX PDU preparation.

use super::ZgfxError;
use super::compressor::Compressor;
use super::wrapper::{ZGFX_SEGMENTED_MAXSIZE, wrap_compressed, wrap_uncompressed};

/// Controls whether ZGFX compression is applied.
///
/// The modes apply to PDUs passed to [`compress_and_wrap_egfx()`]. A caller that
/// knows a PDU is already entropy coded, as the EGFX server does for H.264
/// surface commands, sends it with [`wrap_uncompressed_recorded()`] instead,
/// whatever the mode, so `Always` means every PDU the caller leaves to this
/// module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMode {
    /// Send uncompressed (no CPU overhead).
    ///
    /// Nothing is recorded in the compressor history in this mode, so a [`Compressor`] used with
    /// `Never` must not later be used with `Auto`, `Always` or [`wrap_uncompressed_recorded()`]:
    /// the receiver records every segment, and the histories would no longer agree.
    Never,
    /// Compress and use the smaller result (bandwidth vs CPU trade-off).
    Auto,
    /// Always compress (best bandwidth).
    Always,
}

/// Compress and wrap EGFX PDU bytes into ZGFX segment format for DVC transmission.
///
/// In `Auto` mode, compression is only used when it actually reduces size.
/// The `compressor` maintains history state across calls for back-reference
/// efficiency.
pub fn compress_and_wrap_egfx(
    data: &[u8],
    compressor: &mut Compressor,
    mode: CompressionMode,
) -> Result<Vec<u8>, ZgfxError> {
    match mode {
        CompressionMode::Never => Ok(wrap_uncompressed(data)),
        CompressionMode::Auto => {
            let compressed = compressor.compress(data)?;

            // Only use compressed wrapping if it fits a single segment.
            // Incompressible data can expand beyond the limit; fall back
            // to uncompressed which handles multipart natively.
            if compressed.len() <= ZGFX_SEGMENTED_MAXSIZE {
                let wrapped_compressed = wrap_compressed(&compressed);
                let wrapped_uncompressed = wrap_uncompressed(data);

                if wrapped_compressed.len() < wrapped_uncompressed.len() {
                    Ok(wrapped_compressed)
                } else {
                    Ok(wrapped_uncompressed)
                }
            } else {
                Ok(wrap_uncompressed(data))
            }
        }
        CompressionMode::Always => {
            let compressed = compressor.compress(data)?;

            if compressed.len() <= ZGFX_SEGMENTED_MAXSIZE {
                Ok(wrap_compressed(&compressed))
            } else {
                // Compressed output too large for single segment;
                // send uncompressed to avoid invalid segmentation
                Ok(wrap_uncompressed(data))
            }
        }
    }
}

/// Wrap `data` in uncompressed ZGFX segments and record its bytes in the
/// `compressor` history, for a PDU the caller sends uncompressed on purpose.
///
/// MS-RDPEGFX 3.1.9.1.2 requires every output byte, including the bytes of
/// segments sent uncompressed, to be recorded in the history, because the
/// receiver does so and later back-references point at those bytes. Recording
/// and wrapping happen together here so that neither can be done without the
/// other. Use it, in place of [`compress_and_wrap_egfx()`], for data that is
/// already entropy coded such as H.264, where searching for matches costs CPU
/// and almost never shrinks the PDU.
///
/// The `compressor` history must already hold every byte the receiver has seen
/// on this stream, so don't use this after PDUs sent with
/// [`CompressionMode::Never`] or [`wrap_uncompressed()`], which record nothing.
pub fn wrap_uncompressed_recorded(data: &[u8], compressor: &mut Compressor) -> Vec<u8> {
    compressor.record_uncompressed(data);
    wrap_uncompressed(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_never_produces_uncompressed() {
        let mut compressor = Compressor::new();
        let data = b"Test data";

        let wrapped = compress_and_wrap_egfx(data, &mut compressor, CompressionMode::Never).unwrap();

        assert_eq!(wrapped[0], 0xE0);
        assert_eq!(wrapped[1], 0x04); // RDP8, not compressed
    }

    #[test]
    fn mode_always_produces_compressed() {
        let mut compressor = Compressor::new();
        let data = b"Test data";

        let wrapped = compress_and_wrap_egfx(data, &mut compressor, CompressionMode::Always).unwrap();

        assert_eq!(wrapped[0], 0xE0);
        assert_eq!(wrapped[1], 0x24); // RDP8 + COMPRESSED
    }

    #[test]
    fn mode_auto_compresses_repetitive_data() {
        let mut compressor = Compressor::new();
        let data = b"AAAAAAAAAAAABBBBBBBBBBBBCCCCCCCCCCCC";

        let wrapped = compress_and_wrap_egfx(data, &mut compressor, CompressionMode::Auto).unwrap();

        assert_eq!(wrapped[0], 0xE0);
        assert_eq!(wrapped[1], 0x24);
    }

    #[test]
    fn round_trip_all_modes() {
        use super::super::Decompressor;

        let data = b"Test data with some repetition: AAAA BBBB CCCC";
        let mut decompressor = Decompressor::new();

        for mode in [CompressionMode::Never, CompressionMode::Auto, CompressionMode::Always] {
            let mut compressor = Compressor::new();
            let wrapped = compress_and_wrap_egfx(data, &mut compressor, mode).unwrap();

            let mut output = Vec::new();
            decompressor.decompress(&wrapped, &mut output).unwrap();

            assert_eq!(&output, data, "Round-trip failed for mode {mode:?}");
        }
    }

    /// The receiver records an uncompressed segment in its history, so a PDU sent
    /// through `wrap_uncompressed_recorded()` must be recorded here too, or the
    /// back-reference in the PDU after it points at the wrong bytes.
    #[test]
    fn wrap_uncompressed_recorded_keeps_the_receivers_history_in_step() {
        use super::super::Decompressor;

        let text = b"a tile of text that repeats later in the session, ".repeat(40);
        let mut state: u32 = 0x1234_5678;
        let entropy_coded: Vec<u8> = core::iter::repeat_with(|| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            u8::try_from(state >> 24).unwrap()
        })
        .take(8_000)
        .collect();

        let mut compressor = Compressor::new();
        let mut decompressor = Decompressor::new();
        let mut decoded = Vec::new();

        let first = compress_and_wrap_egfx(&text, &mut compressor, CompressionMode::Always).unwrap();
        decompressor.decompress(&first, &mut decoded).unwrap();
        assert_eq!(decoded, text);

        let second = wrap_uncompressed_recorded(&entropy_coded, &mut compressor);
        assert_eq!(second[1], 0x04, "sent uncompressed");
        decoded.clear();
        decompressor.decompress(&second, &mut decoded).unwrap();
        assert_eq!(decoded, entropy_coded);

        // Reaches back over the recorded bytes to the first PDU, which only decodes
        // correctly when both sides counted the uncompressed segment.
        let third = compress_and_wrap_egfx(&text, &mut compressor, CompressionMode::Always).unwrap();
        assert_eq!(third[1], 0x24, "sent compressed");
        decoded.clear();
        decompressor.decompress(&third, &mut decoded).unwrap();
        assert_eq!(decoded, text);
    }
}
