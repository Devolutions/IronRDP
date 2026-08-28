use ironrdp_core::{Decode as _, ReadCursor};
use ironrdp_egfx::pdu::{Avc420BitmapStream, Avc420Region, align_to_16, annex_b_to_avc, encode_avc420_bitmap_stream};

#[test]
fn avc420_region_full_frame() {
    let region = Avc420Region::full_frame(1920, 1080, 22);
    assert_eq!(region.left, 0);
    assert_eq!(region.top, 0);
    assert_eq!(region.right, 1919);
    assert_eq!(region.bottom, 1079);
    assert_eq!(region.quantization_parameter, 22);
    assert_eq!(region.quality, 100);
}

#[test]
fn align_to_16_rounds_up_to_the_next_multiple() {
    assert_eq!(align_to_16(0), 0);
    assert_eq!(align_to_16(1), 16);
    assert_eq!(align_to_16(15), 16);
    assert_eq!(align_to_16(16), 16);
    assert_eq!(align_to_16(17), 32);
    assert_eq!(align_to_16(1920), 1920);
    assert_eq!(align_to_16(1080), 1088);
}

#[test]
fn annex_b_to_avc_3byte_start_code() {
    // NAL with 3-byte start code: 00 00 01 <NAL>
    let annex_b = [0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E];
    let avc = annex_b_to_avc(&annex_b);

    // Should be: 4-byte length (4) + NAL data
    assert_eq!(avc.len(), 8);
    assert_eq!(&avc[0..4], &[0, 0, 0, 4]); // Length = 4
    assert_eq!(&avc[4..8], &[0x67, 0x42, 0x00, 0x1E]);
}

#[test]
fn annex_b_to_avc_4byte_start_code() {
    // NAL with 4-byte start code: 00 00 00 01 <NAL>
    let annex_b = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00];
    let avc = annex_b_to_avc(&annex_b);

    assert_eq!(avc.len(), 7);
    assert_eq!(&avc[0..4], &[0, 0, 0, 3]); // Length = 3
    assert_eq!(&avc[4..7], &[0x67, 0x42, 0x00]);
}

#[test]
fn annex_b_to_avc_multiple_nals() {
    // Two NAL units
    let annex_b = [
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, // SPS
        0x00, 0x00, 0x01, 0x68, 0xCE, // PPS with 3-byte start
    ];
    let avc = annex_b_to_avc(&annex_b);

    // First NAL: 4 bytes length + 2 bytes data
    // Second NAL: 4 bytes length + 2 bytes data
    assert!(avc.len() >= 12);
}

#[test]
fn annex_b_to_avc_empty_input_yields_empty_output() {
    let avc = annex_b_to_avc(&[]);
    assert!(avc.is_empty());
}

#[test]
fn encode_avc420_bitmap_stream_round_trips_through_decode() {
    let regions = vec![Avc420Region::full_frame(1920, 1080, 22)];
    let h264_data = [0x00, 0x00, 0x00, 0x01, 0x67]; // Minimal H.264

    let encoded = encode_avc420_bitmap_stream(&regions, &h264_data);

    // Should have: 4 bytes (nRect=1) + 8 bytes (rectangle) + 2 bytes (quant) + 5 bytes (data)
    assert_eq!(encoded.len(), 4 + 8 + 2 + 5);

    // Verify we can decode it back
    let mut cursor = ReadCursor::new(&encoded);
    let decoded = Avc420BitmapStream::decode(&mut cursor).expect("decode failed");

    assert_eq!(decoded.rectangles.len(), 1);
    assert_eq!(decoded.quant_qual_vals.len(), 1);
    assert_eq!(decoded.data, &h264_data);
}
