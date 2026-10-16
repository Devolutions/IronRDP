//! High-level ZGFX compression API for EGFX PDU preparation.

use super::ZgfxError;
use super::compressor::Compressor;
use super::wrapper::{ZGFX_SEGMENTED_MAXSIZE, wrap_compressed, wrap_multipart, wrap_uncompressed};

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
///
/// Input longer than 65535 bytes is split into segments that each decompress
/// to at most 65535 bytes, as MS-RDPEGFX 3.1.9.1 requires.
pub fn compress_and_wrap_egfx(
    data: &[u8],
    compressor: &mut Compressor,
    mode: CompressionMode,
) -> Result<Vec<u8>, ZgfxError> {
    if data.len() > ZGFX_SEGMENTED_MAXSIZE && mode != CompressionMode::Never {
        return compress_and_wrap_multipart(data, compressor, mode);
    }

    match mode {
        CompressionMode::Never => Ok(wrap_uncompressed(data)),
        CompressionMode::Auto => {
            let compressed = compressor.compress(data)?;

            // Only use compressed wrapping if it fits a single segment.
            // Incompressible data can expand beyond the limit; fall back
            // to uncompressed.
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

/// Largest growth of a chunk through compression that `Always` mode still accepts.
const ZGFX_MAX_EXPANSION: usize = 1000;

/// Compress input larger than one segment as a multipart message.
///
/// Each segment decompresses to at most `ZGFX_SEGMENTED_MAXSIZE` bytes
/// (MS-RDPEGFX 3.1.9.1). The chunks go through the compressor in order, so its
/// history stays in step with the client, which decompresses segments in order.
/// A chunk whose compressed form is not usable (too large, or in `Auto` mode not
/// smaller, or in `Always` mode grown by more than `ZGFX_MAX_EXPANSION`) is sent
/// raw; the client adds raw segments to its history as well.
fn compress_and_wrap_multipart(
    data: &[u8],
    compressor: &mut Compressor,
    mode: CompressionMode,
) -> Result<Vec<u8>, ZgfxError> {
    let chunks = data.chunks(ZGFX_SEGMENTED_MAXSIZE);
    let mut compressed_chunks = Vec::with_capacity(chunks.len());

    for chunk in chunks.clone() {
        let compressed = compressor.compress(chunk)?;
        let usable = compressed.len() <= ZGFX_SEGMENTED_MAXSIZE
            && match mode {
                CompressionMode::Always => compressed.len() <= chunk.len() + ZGFX_MAX_EXPANSION,
                _ => compressed.len() < chunk.len(),
            };

        compressed_chunks.push(usable.then_some(compressed));
    }

    let segments: Vec<(bool, &[u8])> = chunks
        .zip(&compressed_chunks)
        .map(|(chunk, compressed)| match compressed {
            Some(compressed) => (true, compressed.as_slice()),
            None => (false, chunk),
        })
        .collect();

    Ok(wrap_multipart(data.len(), &segments))
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

    /// Splits a multipart message into `(compressed, payload)` segments.
    fn multipart_segments(wrapped: &[u8]) -> Vec<(bool, &[u8])> {
        assert_eq!(wrapped[0], 0xE1, "expected multipart");
        let count = usize::from(u16::from_le_bytes([wrapped[1], wrapped[2]]));
        let mut pos = 7;
        let mut out = Vec::new();
        for _ in 0..count {
            let size = usize::try_from(u32::from_le_bytes(wrapped[pos..pos + 4].try_into().unwrap())).unwrap();
            let flags = wrapped[pos + 4];
            out.push((flags & 0x20 != 0, &wrapped[pos + 5..pos + 4 + size]));
            pos += 4 + size;
        }
        assert_eq!(pos, wrapped.len());
        out
    }

    /// Decompresses a multipart message one segment at a time and checks that no segment
    /// decompresses to more than 65535 bytes. Returns the segment layout and the output.
    fn decompress_checked(decompressor: &mut super::super::Decompressor, wrapped: &[u8]) -> (Vec<bool>, Vec<u8>) {
        let mut layout = Vec::new();
        let mut output = Vec::new();

        for (compressed, payload) in multipart_segments(wrapped) {
            assert!(payload.len() <= ZGFX_SEGMENTED_MAXSIZE);

            let mut single = vec![0xE0, if compressed { 0x24 } else { 0x04 }];
            single.extend_from_slice(payload);

            let before = output.len();
            decompressor.decompress(&single, &mut output).unwrap();
            assert!(
                output.len() - before <= ZGFX_SEGMENTED_MAXSIZE,
                "segment decompresses too large"
            );
            layout.push(compressed);
        }

        (layout, output)
    }

    fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed;
        core::iter::repeat_with(|| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            u8::try_from(state >> 24).unwrap()
        })
        .take(len)
        .collect()
    }

    #[test]
    fn large_input_is_split_into_segments_within_limit() {
        use super::super::Decompressor;

        // 72 KB, compressible, not trivially repetitive.
        let data: Vec<u8> = (0..72_000u32).map(|i| u8::try_from((i / 7) % 13).unwrap()).collect();
        assert!(data.len() > ZGFX_SEGMENTED_MAXSIZE);

        for mode in [CompressionMode::Auto, CompressionMode::Always] {
            let mut compressor = Compressor::new();
            let mut decompressor = Decompressor::new();

            // Two messages in a row: the second one uses the shared history.
            for _ in 0..2 {
                let wrapped = compress_and_wrap_egfx(&data, &mut compressor, mode).unwrap();
                let (layout, output) = decompress_checked(&mut decompressor, &wrapped);

                assert_eq!(layout.len(), 2, "mode {mode:?}");
                assert!(layout.contains(&true), "mode {mode:?}");
                assert_eq!(output, data, "round trip failed for mode {mode:?}");
            }
        }
    }

    #[test]
    fn large_incompressible_input_round_trips() {
        use super::super::Decompressor;

        let data = pseudo_random(150_000, 0x1234_5678);

        for mode in [CompressionMode::Auto, CompressionMode::Always] {
            let mut compressor = Compressor::new();
            let mut decompressor = Decompressor::new();
            let wrapped = compress_and_wrap_egfx(&data, &mut compressor, mode).unwrap();
            let (layout, output) = decompress_checked(&mut decompressor, &wrapped);

            assert_eq!(layout.len(), 3, "mode {mode:?}");
            assert_eq!(output, data, "round trip failed for mode {mode:?}");
        }
    }

    #[test]
    fn compressed_chunk_may_reference_raw_chunk() {
        use super::super::Decompressor;

        // Chunk 1 is random and sent raw; chunk 2 repeats it, so it is compressed with
        // back-references into the raw bytes.
        let first = pseudo_random(ZGFX_SEGMENTED_MAXSIZE, 42);
        let mut data = first.clone();
        data.extend_from_slice(&first[..30_000]);

        for mode in [CompressionMode::Auto, CompressionMode::Always] {
            let mut compressor = Compressor::new();
            let mut decompressor = Decompressor::new();
            let wrapped = compress_and_wrap_egfx(&data, &mut compressor, mode).unwrap();
            let (layout, output) = decompress_checked(&mut decompressor, &wrapped);

            assert_eq!(layout, [false, true], "mode {mode:?}");
            assert_eq!(output, data, "round trip failed for mode {mode:?}");
        }
    }

    #[test]
    fn always_mode_sends_badly_expanding_chunk_raw() {
        use super::super::Decompressor;

        // The random tail grows by more than 1000 bytes when compressed, but still fits a segment.
        let mut data = vec![0x55; ZGFX_SEGMENTED_MAXSIZE];
        data.extend_from_slice(&pseudo_random(20_000, 7));

        let mut compressor = Compressor::new();
        let mut decompressor = Decompressor::new();
        let wrapped = compress_and_wrap_egfx(&data, &mut compressor, CompressionMode::Always).unwrap();
        let (layout, output) = decompress_checked(&mut decompressor, &wrapped);

        assert_eq!(layout, [true, false]);
        assert_eq!(output, data);
    }
}
