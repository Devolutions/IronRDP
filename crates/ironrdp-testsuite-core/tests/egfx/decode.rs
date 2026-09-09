use ironrdp_egfx::decode::{H264Decoder, OpenH264Decoder};

// ============================================================================
// Test Helpers
// ============================================================================

/// Generate a minimal AVC-format H.264 bitstream by encoding a black 16x16 frame
///
/// The encoder produces Annex B format (start code prefixed). This function
/// converts the output to AVC format (4-byte BE length prefixed) to exercise
/// the full decode pipeline including AVC-to-Annex-B conversion.
fn generate_test_avc_bitstream() -> Vec<u8> {
    use openh264::encoder::Encoder;
    use openh264::formats::YUVBuffer;

    let mut encoder = Encoder::new().expect("encoder should initialize");

    // Black 16x16 YUV420p frame (all zeros)
    let yuv = YUVBuffer::new(16, 16);
    let bitstream = encoder.encode(&yuv).expect("encode should succeed");
    let annex_b = bitstream.to_vec();

    annex_b_to_avc(&annex_b)
}

/// Convert Annex B format NAL units to AVC format (4-byte BE length prefix)
fn annex_b_to_avc(data: &[u8]) -> Vec<u8> {
    let mut avc = Vec::new();
    let mut i = 0;

    // Find NAL unit boundaries by scanning for start codes
    let mut nal_starts = Vec::new();
    while i < data.len() {
        if i + 3 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            nal_starts.push(i + 4);
            i += 4;
        } else if i + 2 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            nal_starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }

    for (idx, &start) in nal_starts.iter().enumerate() {
        let end = if idx + 1 < nal_starts.len() {
            // Find the start code before the next NAL
            let next_start = nal_starts[idx + 1];
            // Back up past the start code prefix
            if next_start >= 4 && data[next_start - 4] == 0 && data[next_start - 3] == 0 && data[next_start - 2] == 0 {
                next_start - 4
            } else {
                next_start - 3
            }
        } else {
            data.len()
        };

        let nal_data = &data[start..end];

        #[expect(clippy::as_conversions, reason = "NAL unit length for test data")]
        let len = nal_data.len() as u32;
        avc.extend_from_slice(&len.to_be_bytes());
        avc.extend_from_slice(nal_data);
    }

    avc
}

// ============================================================================
// Happy Path Tests
// ============================================================================

#[test]
fn test_openh264_decoder_init() {
    let _decoder = OpenH264Decoder::new().expect("decoder should initialize");
}

#[test]
fn test_openh264_decode_sps_pps() {
    // Generate a full bitstream (SPS + PPS + IDR) and verify decode succeeds.
    // SPS and PPS are always delivered together with the first I-frame
    // in RFX_AVC420_BITMAP_STREAM payloads.
    let avc_data = generate_test_avc_bitstream();
    assert!(!avc_data.is_empty(), "encoder should produce output");

    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");
    let frame = decoder.decode(&avc_data).expect("decode should succeed");
    assert!(frame.width() >= 16, "decoded width should be at least 16");
    assert!(frame.height() >= 16, "decoded height should be at least 16");
}

#[test]
fn test_openh264_decode_iframe() {
    let avc_data = generate_test_avc_bitstream();

    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");
    let frame = decoder.decode(&avc_data).expect("decode should succeed");

    // Verify RGBA output dimensions and data
    assert_eq!(frame.width(), 16);
    assert_eq!(frame.height(), 16);
    assert_eq!(frame.data().len(), 16 * 16 * 4, "RGBA data should be 16x16x4 bytes");
}

#[test]
fn test_openh264_decoder_reset() {
    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");

    // Decode a frame to populate internal state
    let avc_data = generate_test_avc_bitstream();
    let _ = decoder.decode(&avc_data);

    // Reset should not panic
    decoder.reset();

    // Decoder should still be usable after reset
    let frame = decoder.decode(&avc_data).expect("decode after reset should succeed");
    assert_eq!(frame.width(), 16);
    assert_eq!(frame.height(), 16);
}

// ============================================================================
// Error Path Tests
// ============================================================================

#[test]
fn test_decode_empty_input() {
    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");

    // Empty input has no NAL units -- the AVC-to-Annex-B converter
    // produces nothing, and OpenH264 returns no picture.
    let result = decoder.decode(&[]);
    assert!(result.is_err(), "decoding empty input should fail");
}

#[test]
fn test_decode_truncated_nal_length() {
    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");

    // Less than 4 bytes: can't even read the NAL length prefix.
    // The converter produces an empty Annex B buffer.
    let result = decoder.decode(&[0x00, 0x00]);
    assert!(result.is_err(), "truncated NAL length should fail");
}

#[test]
fn test_decode_nal_length_exceeds_buffer() {
    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");

    // NAL length says 100 bytes but only 2 bytes follow.
    // The converter discards the malformed NAL and produces empty output.
    let mut data = Vec::new();
    data.extend_from_slice(&100u32.to_be_bytes());
    data.extend_from_slice(&[0x67, 0x00]); // Partial NAL
    let result = decoder.decode(&data);
    assert!(result.is_err(), "oversized NAL length should fail");
}

#[test]
fn test_decode_garbage_input() {
    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");

    // Valid AVC framing (length prefix) but garbage NAL content.
    // OpenH264 should either return an error or no picture.
    let nal_data = [0xFF, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA];
    let mut data = Vec::new();

    #[expect(clippy::as_conversions, reason = "test data length")]
    let len = nal_data.len() as u32;
    data.extend_from_slice(&len.to_be_bytes());
    data.extend_from_slice(&nal_data);

    let result = decoder.decode(&data);
    // OpenH264 may return Ok(no picture) which our decoder converts to an error,
    // or it may return a decode error directly. Either way it shouldn't succeed
    // with a valid frame.
    assert!(result.is_err(), "garbage NAL content should not produce a valid frame");
}

// ============================================================================
// Color Range Tests
// ============================================================================

/// Encode a YUV420p frame with the given luma/chroma planes and dimensions
/// and decode it back through `OpenH264Decoder`, returning the RGBA output.
fn encode_and_decode_yuv(
    planes: (&[u8], &[u8], &[u8]),
    dimensions: (usize, usize),
    strides: (usize, usize, usize),
) -> ironrdp_egfx::decode::DecodedFrame {
    use openh264::encoder::Encoder;
    use openh264::formats::YUVSlices;

    let mut encoder = Encoder::new().expect("encoder should initialize");
    let yuv = YUVSlices::new(planes, dimensions, strides);
    let bitstream = encoder.encode(&yuv).expect("encode should succeed").to_vec();

    let avc_data = annex_b_to_avc(&bitstream);
    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");
    decoder.decode(&avc_data).expect("decode should succeed")
}

#[test]
fn test_decode_preserves_full_luma_range() {
    // MS-RDPEGFX AVC420 is full range, so luma must pass through the
    // YUV-to-RGBA conversion undistorted. A limited-range (studio swing)
    // conversion over-expands: near-white luma is pulled past 255 and
    // clamps there, near-black luma is pulled below 0 and clamps there.
    // Values right at 255/0 survive either conversion after clamping, so
    // the discriminating cases sit just inside the extremes.
    // 16x16 with padded strides: the Y plane stride (24) exceeds its
    // visible width, exercising the macroblock-alignment handling that
    // prevents diagonal streaks; chroma stays at half resolution.
    let (y, u, v) = ([245u8; 16 * 24], [128u8; 8 * 12], [128u8; 8 * 12]);
    let near_white = encode_and_decode_yuv((&y, &u, &v), (16, 16), (24, 12, 12));
    let data = near_white.data();
    for px in data.chunks_exact(4) {
        // A limited-range conversion clamps this to 255.
        assert!(px[0] < 252, "near-white R channel saturated: {}", px[0]);
        assert!(px[1] < 252, "near-white G channel saturated: {}", px[1]);
        assert!(px[2] < 252, "near-white B channel saturated: {}", px[2]);
        assert_eq!(px[3], 255);
    }

    let (y, u, v) = ([10u8; 16 * 24], [128u8; 8 * 12], [128u8; 8 * 12]);
    let near_black = encode_and_decode_yuv((&y, &u, &v), (16, 16), (24, 12, 12));
    let data = near_black.data();
    for px in data.chunks_exact(4) {
        // A limited-range conversion clamps this to 0.
        assert!(px[0] > 4, "near-black R channel crushed: {}", px[0]);
        assert!(px[1] > 4, "near-black G channel crushed: {}", px[1]);
        assert!(px[2] > 4, "near-black B channel crushed: {}", px[2]);
    }
}

#[test]
fn test_decode_uses_bt709_chroma_matrix() {
    // Neutral chroma cannot distinguish BT.601 from BT.709; both matrices
    // agree on grayscale. Encode a saturated red tile (YUV planes fed
    // directly, bypassing the encoder's own RGB conversion) and verify the
    // decoded pixels sit near pure red under the spec's BT.709 inverse
    // (MS-RDPEGFX 3.3.8.3.1, image14). Under a full-range BT.601 inverse
    // the same planes decode with R at roughly 231, failing the R bound
    // (the G bound does not discriminate there: 601's G goes negative and
    // clamps to 0).
    //
    // BT.709 full-range forward on pure red: Y = (54·255) >> 8 = 53,
    // U = (-29·255) >> 8 + 128 = 99, V = (128·255) >> 8 + 128 = 255.
    let y = [53u8; 16 * 16];
    let u = [99u8; 8 * 8];
    let v = [255u8; 8 * 8];
    let red = encode_and_decode_yuv((&y, &u, &v), (16, 16), (16, 8, 8));
    let data = red.data();
    for px in data.chunks_exact(4) {
        assert!(px[0] > 245, "red R channel: {}", px[0]);
        assert!(px[1] < 20, "red G channel leaked: {}", px[1]);
        assert!(px[2] < 20, "red B channel leaked: {}", px[2]);
    }
}
