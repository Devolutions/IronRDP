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
    /// Always compress (best bandwidth), except a segment that compression would grow by
    /// more than 1,000 bytes (MS-RDPEGFX 3.1.9.1.2), which is sent uncompressed.
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

    if mode == CompressionMode::Never {
        return Ok(wrap_uncompressed(data));
    }

    let compressed = compressor.compress(data)?;

    if is_usable(&compressed, data, mode) {
        Ok(wrap_compressed(&compressed))
    } else {
        Ok(wrap_uncompressed(data))
    }
}

/// Maximum expansion of a segment when compressed (MS-RDPEGFX 3.1.9.1.2, RDP 8.0 compressor limits).
const ZGFX_MAX_EXPANSION: usize = 1000;

/// Whether the compressed form of `raw` can be sent in place of it.
///
/// It must fit a segment and stay within the spec's expansion limit. In `Auto`
/// mode it must also be smaller. Otherwise the raw bytes are sent; the client
/// adds raw segments to its history as well.
fn is_usable(compressed: &[u8], raw: &[u8], mode: CompressionMode) -> bool {
    compressed.len() <= ZGFX_SEGMENTED_MAXSIZE
        && compressed.len() <= raw.len() + ZGFX_MAX_EXPANSION
        && (mode == CompressionMode::Always || compressed.len() < raw.len())
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

/// Compress input larger than one segment as a multipart message.
///
/// Each segment decompresses to at most `ZGFX_SEGMENTED_MAXSIZE` bytes
/// (MS-RDPEGFX 3.1.9.1). The chunks go through the compressor in order, so its
/// history stays in step with the client, which decompresses segments in order.
/// Each chunk is sent compressed or raw with the same rules as a single segment
/// (see `is_usable`).
fn compress_and_wrap_multipart(
    data: &[u8],
    compressor: &mut Compressor,
    mode: CompressionMode,
) -> Result<Vec<u8>, ZgfxError> {
    let chunks = data.chunks(ZGFX_SEGMENTED_MAXSIZE);
    let mut compressed_chunks = Vec::with_capacity(chunks.len());

    for chunk in chunks.clone() {
        let compressed = compressor.compress(chunk)?;
        let usable = is_usable(&compressed, chunk, mode);
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
        let entropy_coded = pseudo_random(8_000, 0x1234_5678);

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

    /// Decompresses a multipart message one segment at a time with `per_segment` and checks that
    /// no segment decompresses to more than 65535 bytes, and decompresses the whole message with
    /// `whole`, as a client does, which must give the same output. Returns the segment layout and
    /// the output.
    fn decompress_checked(
        per_segment: &mut super::super::Decompressor,
        whole: &mut super::super::Decompressor,
        wrapped: &[u8],
    ) -> (Vec<bool>, Vec<u8>) {
        let mut layout = Vec::new();
        let mut output = Vec::new();

        for (compressed, payload) in multipart_segments(wrapped) {
            assert!(payload.len() <= ZGFX_SEGMENTED_MAXSIZE);

            let single = if compressed {
                wrap_compressed(payload)
            } else {
                wrap_uncompressed(payload)
            };

            let before = output.len();
            per_segment.decompress(&single, &mut output).unwrap();
            assert!(
                output.len() - before <= ZGFX_SEGMENTED_MAXSIZE,
                "segment decompresses too large"
            );
            layout.push(compressed);
        }

        let mut whole_output = Vec::new();
        whole.decompress(wrapped, &mut whole_output).unwrap();
        assert_eq!(whole_output, output, "whole message decompresses differently");

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
            let mut whole = Decompressor::new();

            // Two messages in a row: the second one uses the shared history.
            for _ in 0..2 {
                let wrapped = compress_and_wrap_egfx(&data, &mut compressor, mode).unwrap();
                let (layout, output) = decompress_checked(&mut decompressor, &mut whole, &wrapped);

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
            let mut whole = Decompressor::new();
            let wrapped = compress_and_wrap_egfx(&data, &mut compressor, mode).unwrap();
            let (layout, output) = decompress_checked(&mut decompressor, &mut whole, &wrapped);

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
            let mut whole = Decompressor::new();
            let wrapped = compress_and_wrap_egfx(&data, &mut compressor, mode).unwrap();
            let (layout, output) = decompress_checked(&mut decompressor, &mut whole, &wrapped);

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
        let mut whole = Decompressor::new();
        let wrapped = compress_and_wrap_egfx(&data, &mut compressor, CompressionMode::Always).unwrap();
        let (layout, output) = decompress_checked(&mut decompressor, &mut whole, &wrapped);

        assert_eq!(layout, [true, false]);
        assert_eq!(output, data);
    }

    #[test]
    fn always_mode_sends_badly_expanding_single_segment_raw() {
        use super::super::Decompressor;

        // Grows by more than 1000 bytes when compressed, but still fits a segment.
        let data = pseudo_random(20_000, 7);

        let mut compressor = Compressor::new();
        let mut decompressor = Decompressor::new();
        let wrapped = compress_and_wrap_egfx(&data, &mut compressor, CompressionMode::Always).unwrap();
        assert_eq!(wrapped[1], 0x04, "sent uncompressed");

        let mut output = Vec::new();
        decompressor.decompress(&wrapped, &mut output).unwrap();
        assert_eq!(output, data);
    }
}
