use ironrdp_egfx::decode::{H264Decoder as _, OpenH264Decoder};
use ironrdp_egfx::encode::{EncodeFrame, H264Encoder as _, OpenH264Encoder};
use ironrdp_egfx::pdu::annex_b_to_avc;

// ============================================================================
// Color Range Tests
// ============================================================================

/// Encode a 16x16 RGBA frame with the given solid color through
/// `OpenH264Encoder`, decode it back with a spec-conformant full-range
/// BT.709 decoder, and return the decoded RGBA buffer.
fn encode_and_decode_solid(rgba: [u8; 4]) -> Vec<u8> {
    let mut encoder = OpenH264Encoder::new().expect("encoder should initialize");
    let mut data = Vec::with_capacity(16 * 16 * 4);
    for _ in 0..16 * 16 {
        data.extend_from_slice(&rgba);
    }
    let frame = EncodeFrame {
        data: &data,
        width: 16,
        height: 16,
    };
    let bitstream = encoder.encode(frame).expect("encode should succeed");
    let avc = annex_b_to_avc(&bitstream);

    let mut decoder = OpenH264Decoder::new().expect("decoder should initialize");
    let decoded = decoder.decode(&avc).expect("decode should succeed");
    decoded.into_data()
}

#[test]
fn test_encode_round_trips_saturated_colors() {
    // The encoder must place luma across the full [0, 255] swing with
    // BT.709 chroma so that a spec-conformant decoder (full-range BT.709,
    // per MS-RDPEGFX 3.3.8.3.1) reconstructs saturated primaries near
    // their channel extremes. Under the previous limited-range BT.601
    // conversion, white luma lands at 235 and saturated primaries shift.
    //
    // The cold-channel bounds distinguish the BT.709 matrix from a
    // hypothetical full-range BT.601 regression: the two matrices place
    // primary-color energy differently across the non-hot channels.
    let cases: [([u8; 4], usize, bool); 5] = [
        ([255, 255, 255, 255], 0, true), // white: every channel near 255
        ([0, 0, 0, 255], 0, false),      // black: every channel near 0
        ([255, 0, 0, 255], 0, true),     // red: R near 255, G/B near 0
        ([0, 255, 0, 255], 1, true),     // green: G near 255, R/B near 0
        ([0, 0, 255, 255], 2, true),     // blue: B near 255, R/G near 0
    ];
    for (rgba, hot, above) in cases {
        let data = encode_and_decode_solid(rgba);
        assert_eq!(data.len(), 16 * 16 * 4);
        for px in data.chunks_exact(4) {
            if above {
                assert!(px[hot] > 245, "color {rgba:?}: hot channel {hot} = {}", px[hot]);
            } else {
                assert!(px[hot] < 10, "color {rgba:?}: hot channel {hot} = {}", px[hot]);
            }
            assert_eq!(px[3], 255, "alpha must be opaque");
        }
    }
}

#[test]
fn test_encode_round_trip_cold_channels_stay_cold() {
    // Cross-channel leakage discriminates the BT.709 chroma matrix from
    // full-range BT.601: encoding pure red with BT.601 coefficients and
    // decoding with the spec's BT.709 inverse yields G≈24 on a clean
    // tile, while correct BT.709 round-tripping stays near the AVC
    // quantization floor. The bound must stay tight enough to expose
    // that gap but loose enough for codec noise on a 16x16 tile.
    let cases: [([u8; 4], [usize; 2]); 3] = [
        ([255, 0, 0, 255], [1, 2]), // red: G and B stay near 0
        ([0, 255, 0, 255], [0, 2]), // green: R and B stay near 0
        ([0, 0, 255, 255], [0, 1]), // blue: R and G stay near 0
    ];
    for (rgba, cold) in cases {
        let data = encode_and_decode_solid(rgba);
        for px in data.chunks_exact(4) {
            for c in cold {
                assert!(px[c] < 20, "color {rgba:?}: cold channel {c} leaked: {}", px[c]);
            }
        }
    }
}
