use ironrdp_core::ReadCursor;
use ironrdp_graphics::clearcodec::{ClearCodecDecoder, ClearCodecEncoder};
use ironrdp_pdu::codecs::clearcodec::{
    ClearCodecBitmapStream, FLAG_CACHE_RESET, FLAG_GLYPH_HIT, FLAG_GLYPH_INDEX, RgbRunSegment, encode_residual_layer,
};

// ============================================================================
// Helpers
// ============================================================================

/// Build a residual-only ClearCodec stream (no bands, no subcodec).
fn make_residual_stream(seq: u8, flags: u8, glyph_index: Option<u16>, residual: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    data.push(flags);
    data.push(seq);
    if let Some(idx) = glyph_index {
        data.extend_from_slice(&idx.to_le_bytes());
    }
    let residual_len = u32::try_from(residual.len()).unwrap();
    data.extend_from_slice(&residual_len.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes()); // bands
    data.extend_from_slice(&0u32.to_le_bytes()); // subcodec
    data.extend_from_slice(residual);
    data
}

/// Build a solid-color residual payload for width*height pixels.
fn make_solid_residual(b: u8, g: u8, r: u8, pixel_count: u32) -> Vec<u8> {
    encode_residual_layer(&[RgbRunSegment {
        blue: b,
        green: g,
        red: r,
        run_length: pixel_count,
    }])
}

/// Build BGRA pixel data for a solid color.
fn solid_bgra(b: u8, g: u8, r: u8, pixel_count: usize) -> Vec<u8> {
    (0..pixel_count).flat_map(|_| [b, g, r, 0xFF]).collect()
}

// ============================================================================
// Codec Round-Trip (encode -> decode, pixel-perfect)
// ============================================================================

#[test]
fn round_trip_1x1_single_pixel() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0x00, 0x00, 0x00, 1);
    let wire = enc.encode(&bgra, 1, 1);
    let result = dec.decode(&wire, 1, 1).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_4x4_solid_color() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0x00, 0x00, 0xFF, 16);
    let wire = enc.encode(&bgra, 4, 4);
    let result = dec.decode(&wire, 4, 4).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_checkerboard_alternating_pixels() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let mut bgra = Vec::with_capacity(16 * 4);
    for i in 0..16 {
        if i % 2 == 0 {
            bgra.extend_from_slice(&[0x00, 0x00, 0x00, 0xFF]);
        } else {
            bgra.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        }
    }
    let wire = enc.encode(&bgra, 4, 4);
    let result = dec.decode(&wire, 4, 4).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_8x1_all_unique_colors() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra: Vec<u8> = (0..8u8).flat_map(|i| [i * 30, i * 20, i * 10, 0xFF]).collect();
    let wire = enc.encode(&bgra, 8, 1);
    let result = dec.decode(&wire, 8, 1).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_100x100_triggers_medium_run_encoding() {
    // 10,000 pixels requires factor2 (u16) encoding tier in residual layer
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0x42, 0x84, 0xC6, 10_000);
    let wire = enc.encode(&bgra, 100, 100);
    let result = dec.decode(&wire, 100, 100).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_asymmetric_1x1000() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0xAB, 0xCD, 0xEF, 1000);
    let wire = enc.encode(&bgra, 1, 1000);
    let result = dec.decode(&wire, 1, 1000).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_asymmetric_1000x1() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0x11, 0x22, 0x33, 1000);
    let wire = enc.encode(&bgra, 1000, 1);
    let result = dec.decode(&wire, 1000, 1).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_at_glyph_cache_boundary_1024_pixels() {
    // 32x32 = 1024 pixels: maximum size eligible for glyph caching
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0x80, 0x80, 0x80, 1024);
    let wire = enc.encode(&bgra, 32, 32);
    let result = dec.decode(&wire, 32, 32).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_over_glyph_threshold_no_caching() {
    // 33x32 = 1056 pixels: too large for glyph caching
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0x80, 0x80, 0x80, 1056);
    let wire = enc.encode(&bgra, 33, 32);
    let result = dec.decode(&wire, 33, 32).unwrap();
    assert_eq!(result, bgra);
}

#[test]
fn round_trip_two_color_stripe() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let mut bgra = Vec::new();
    for _ in 0..50 {
        bgra.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]);
    }
    for _ in 0..50 {
        bgra.extend_from_slice(&[0xFF, 0x00, 0x00, 0xFF]);
    }
    let wire = enc.encode(&bgra, 100, 1);
    let result = dec.decode(&wire, 100, 1).unwrap();
    assert_eq!(result, bgra);
}

// ============================================================================
// Adversarial Input (no panic, no hang, correct errors)
// ============================================================================

#[test]
fn adversarial_residual_max_run_length_completes_quickly() {
    // run_length = u32::MAX in a 1x1 surface: must not spin for 4B iterations
    let mut dec = ClearCodecDecoder::new();
    let residual = [0xFF, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let stream = make_residual_stream(0, 0, None, &residual);
    let result = dec.decode(&stream, 1, 1).unwrap();
    assert_eq!(result.len(), 4);
    assert_eq!(&result[..3], &[0xFF, 0x00, 0x00]); // BGR written correctly
}

#[test]
fn adversarial_residual_zero_run_produces_empty_output() {
    let mut dec = ClearCodecDecoder::new();
    let residual = [0xFF, 0x00, 0x00, 0x00]; // run = 0
    let stream = make_residual_stream(0, 0, None, &residual);
    let result = dec.decode(&stream, 1, 1).unwrap();
    assert_eq!(result, vec![0; 4]); // output stays zeroed
}

#[test]
fn adversarial_glyph_hit_for_uncached_index() {
    let mut dec = ClearCodecDecoder::new();
    let mut data = vec![FLAG_GLYPH_INDEX | FLAG_GLYPH_HIT, 0x00];
    data.extend_from_slice(&42u16.to_le_bytes());
    assert!(dec.decode(&data, 1, 1).is_err());
}

#[test]
fn adversarial_glyph_hit_without_glyph_index_flag() {
    let mut dec = ClearCodecDecoder::new();
    let data = [FLAG_GLYPH_HIT, 0x00];
    assert!(dec.decode(&data, 1, 1).is_err());
}

#[test]
fn adversarial_glyph_index_out_of_spec_range() {
    let mut dec = ClearCodecDecoder::new();
    // glyphIndex = 4000 (spec requires 0-3999)
    let residual = make_solid_residual(0, 0, 0, 1);
    let stream = make_residual_stream(0, FLAG_GLYPH_INDEX, Some(4000), &residual);
    assert!(dec.decode(&stream, 1, 1).is_err());
}

#[test]
fn adversarial_glyph_index_max_u16() {
    let mut dec = ClearCodecDecoder::new();
    let residual = make_solid_residual(0, 0, 0, 1);
    let stream = make_residual_stream(0, FLAG_GLYPH_INDEX, Some(u16::MAX), &residual);
    assert!(dec.decode(&stream, 1, 1).is_err());
}

#[test]
fn adversarial_composite_byte_count_overflow() {
    let mut data = vec![0x00, 0x00]; // flags, seq
    // residualByteCount + bandsByteCount overflows usize
    data.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    let mut cursor = ReadCursor::new(&data);
    assert!(ClearCodecBitmapStream::decode(&mut cursor).is_err());
}

#[test]
fn adversarial_sequence_number_wraps_at_256() {
    let mut dec = ClearCodecDecoder::new();
    // Drive sequence through 0..255 and back to 0 (wrapping)
    for seq in 0..=255u8 {
        let residual = make_solid_residual(0, 0, 0, 1);
        let stream = make_residual_stream(seq, 0, None, &residual);
        dec.decode(&stream, 1, 1).unwrap();
    }
    // Wrap back to 0
    let residual = make_solid_residual(0, 0, 0, 1);
    let stream = make_residual_stream(0, 0, None, &residual);
    dec.decode(&stream, 1, 1).unwrap();
}

#[test]
fn adversarial_stream_truncated_to_1_byte() {
    let data = [0x00];
    let mut cursor = ReadCursor::new(&data);
    assert!(ClearCodecBitmapStream::decode(&mut cursor).is_err());
}

#[test]
fn adversarial_stream_empty() {
    let data = [];
    let mut cursor = ReadCursor::new(&data);
    assert!(ClearCodecBitmapStream::decode(&mut cursor).is_err());
}

// ============================================================================
// Cache State Management
// ============================================================================

#[test]
fn glyph_cache_store_then_hit() {
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0xFF, 0x00, 0x00, 1);

    // Frame 1: store glyph at index 42
    let residual = make_solid_residual(0xFF, 0x00, 0x00, 1);
    let stream1 = make_residual_stream(0, FLAG_GLYPH_INDEX, Some(42), &residual);
    let p1 = dec.decode(&stream1, 1, 1).unwrap();
    assert_eq!(p1, bgra);

    // Frame 2: glyph hit at index 42
    let mut stream2 = vec![FLAG_GLYPH_INDEX | FLAG_GLYPH_HIT, 0x01];
    stream2.extend_from_slice(&42u16.to_le_bytes());
    let p2 = dec.decode(&stream2, 1, 1).unwrap();
    assert_eq!(p2, bgra);
}

#[test]
fn glyph_cache_overwrite_at_same_index() {
    let mut dec = ClearCodecDecoder::new();

    // Store red at index 0
    let red_residual = make_solid_residual(0x00, 0x00, 0xFF, 1);
    let stream1 = make_residual_stream(0, FLAG_GLYPH_INDEX, Some(0), &red_residual);
    dec.decode(&stream1, 1, 1).unwrap();

    // Overwrite with blue at index 0
    let blue_residual = make_solid_residual(0xFF, 0x00, 0x00, 1);
    let stream2 = make_residual_stream(1, FLAG_GLYPH_INDEX, Some(0), &blue_residual);
    dec.decode(&stream2, 1, 1).unwrap();

    // Hit should return blue
    let mut stream3 = vec![FLAG_GLYPH_INDEX | FLAG_GLYPH_HIT, 0x02];
    stream3.extend_from_slice(&0u16.to_le_bytes());
    let result = dec.decode(&stream3, 1, 1).unwrap();
    assert_eq!(result[0], 0xFF); // blue channel
}

#[test]
fn cache_reset_does_not_panic() {
    let mut dec = ClearCodecDecoder::new();
    let residual = make_solid_residual(0, 0, 0, 1);
    let stream1 = make_residual_stream(0, 0, None, &residual);
    dec.decode(&stream1, 1, 1).unwrap();

    let stream2 = [FLAG_CACHE_RESET, 0x01];
    let _ = dec.decode(&stream2, 0, 0);
}

#[test]
fn encoder_glyph_hit_produces_smaller_output() {
    let mut enc = ClearCodecEncoder::new();
    let bgra = solid_bgra(0xAA, 0xBB, 0xCC, 1);

    let first = enc.encode(&bgra, 1, 1);
    let second = enc.encode(&bgra, 1, 1); // should be glyph hit

    assert!(second.len() < first.len(), "glyph hit should be smaller");
}

#[test]
fn encoder_glyph_miss_after_content_change() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let red = solid_bgra(0x00, 0x00, 0xFF, 1);
    let blue = solid_bgra(0xFF, 0x00, 0x00, 1);

    let first = enc.encode(&red, 1, 1);
    let second = enc.encode(&blue, 1, 1); // different content, full encode

    // Verify both are full encodes (not glyph hits) by checking they
    // contain a composite header (minimum 14 bytes: flags + seq + 3*u32)
    assert!(second.len() >= 14, "changed content should produce a full encode");

    // Verify they decode to the correct distinct colors
    let decoded_red = dec.decode(&first, 1, 1).unwrap();
    let decoded_blue = dec.decode(&second, 1, 1).unwrap();
    assert_eq!(decoded_red, red);
    assert_eq!(decoded_blue, blue);
}

#[test]
fn encoder_sequence_numbers_increment_correctly() {
    let mut enc = ClearCodecEncoder::new();
    let bgra = solid_bgra(0, 0, 0, 1);

    let e1 = enc.encode(&bgra, 1, 1);
    let e2 = enc.encode(&bgra, 1, 1); // glyph hit
    let e3 = enc.encode(&solid_bgra(0xFF, 0xFF, 0xFF, 1), 1, 1); // different

    assert_eq!(e1[1], 0); // seq byte at offset 1
    assert_eq!(e2[1], 1);
    assert_eq!(e3[1], 2);
}

#[test]
fn encoder_cache_reset_round_trips() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let reset = enc.encode_cache_reset();
    let _ = dec.decode(&reset, 0, 0);
}

// ============================================================================
// Compression Quality
// ============================================================================

#[test]
fn solid_color_compresses_below_30_bytes() {
    let mut enc = ClearCodecEncoder::new();
    let bgra = solid_bgra(0x42, 0x84, 0xC6, 10_000);
    let wire = enc.encode(&bgra, 100, 100);
    // 10,000 pixels = 40,000 bytes raw. Solid color: header + 1 run segment.
    assert!(
        wire.len() < 30,
        "solid 100x100 should compress to <30 bytes, got {}",
        wire.len()
    );
}

#[test]
fn unique_pixels_do_not_expand_beyond_raw() {
    let mut enc = ClearCodecEncoder::new();
    let bgra: Vec<u8> = (0..100u8)
        .flat_map(|i| [i, i.wrapping_mul(2), i.wrapping_mul(3), 0xFF])
        .collect();
    let wire = enc.encode(&bgra, 100, 1);
    // Worst case: each pixel is unique, 1 segment per pixel.
    // Should not be larger than raw + header overhead.
    assert!(wire.len() < bgra.len() + 50);
}

// ============================================================================
// Multi-Frame Session Simulation
// ============================================================================

#[test]
fn session_10_frames_mixed_colors() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();

    let colors: Vec<(u8, u8, u8)> = vec![
        (0, 0, 0),
        (0xFF, 0, 0),
        (0, 0xFF, 0),
        (0, 0, 0xFF),
        (0xFF, 0xFF, 0),
        (0xFF, 0, 0xFF),
        (0, 0xFF, 0xFF),
        (0x80, 0x80, 0x80),
        (0xFF, 0xFF, 0xFF),
        (0, 0, 0),
    ];

    for (b, g, r) in &colors {
        let bgra = solid_bgra(*b, *g, *r, 4);
        let wire = enc.encode(&bgra, 2, 2);
        let result = dec.decode(&wire, 2, 2).unwrap();
        assert_eq!(result, bgra);
    }
}

#[test]
fn session_repeated_frames_hit_glyph_cache() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();
    let bgra = solid_bgra(0xDE, 0xAD, 0xBE, 4);

    let wire1 = enc.encode(&bgra, 2, 2);
    let len1 = wire1.len();
    dec.decode(&wire1, 2, 2).unwrap();

    // Subsequent encodes should be glyph hits (smaller)
    for _ in 0..5 {
        let wire = enc.encode(&bgra, 2, 2);
        assert!(wire.len() < len1, "repeated frame should use glyph cache");
        let result = dec.decode(&wire, 2, 2).unwrap();
        assert_eq!(result, bgra);
    }
}

#[test]
fn session_encoder_decoder_stay_synchronized_across_50_frames() {
    let mut enc = ClearCodecEncoder::new();
    let mut dec = ClearCodecDecoder::new();

    for i in 0u8..50 {
        let bgra = solid_bgra(i, i.wrapping_mul(3), i.wrapping_mul(7), 9);
        let wire = enc.encode(&bgra, 3, 3);
        let result = dec.decode(&wire, 3, 3).unwrap();
        assert_eq!(result, bgra, "mismatch at frame {i}");
    }
}

// ============================================================================
// Bands Layer Compositing (integration through decoder)
// ============================================================================

#[test]
fn decode_stream_with_bands_layer_short_vbar_cache_miss() {
    // Construct a minimal ClearCodec stream with a bands layer containing
    // one band, one column, using a ShortVBarCacheMiss. This exercises the
    // full decode_composite -> resolve_vbar -> blit path.
    let mut dec = ClearCodecDecoder::new();

    // Surface: 4 pixels wide, 4 pixels tall
    let width: u16 = 4;
    let height: u16 = 4;

    // Build bands layer data: one band covering column 1, rows 0-3
    let mut bands_data = Vec::new();
    bands_data.extend_from_slice(&1u16.to_le_bytes()); // x_start = 1
    bands_data.extend_from_slice(&1u16.to_le_bytes()); // x_end = 1 (1 column)
    bands_data.extend_from_slice(&0u16.to_le_bytes()); // y_start = 0
    bands_data.extend_from_slice(&3u16.to_le_bytes()); // y_end = 3 (height = 4)
    bands_data.extend_from_slice(&[0x00, 0x00, 0x00]); // background BGR = black

    // V-bar: ShortCacheMiss with y_on=1, y_off=3 (2 pixels at rows 1-2)
    // bits 13:6 = y_on (1), bits 5:0 = y_off (3)
    let vbar_word: u16 = (1 << 6) | 3;
    bands_data.extend_from_slice(&vbar_word.to_le_bytes());
    // 2 pixels * 3 bytes = 6 bytes of BGR pixel data (red)
    bands_data.extend_from_slice(&[0x00, 0x00, 0xFF]); // row 1: red
    bands_data.extend_from_slice(&[0x00, 0x00, 0xFF]); // row 2: red

    // Build the full stream: no residual, bands only, no subcodec
    let mut stream = Vec::new();
    stream.push(0x00); // flags
    stream.push(0x00); // seq
    stream.extend_from_slice(&0u32.to_le_bytes()); // residualByteCount = 0
    let bands_len = u32::try_from(bands_data.len()).unwrap();
    stream.extend_from_slice(&bands_len.to_le_bytes()); // bandsByteCount
    stream.extend_from_slice(&0u32.to_le_bytes()); // subcodecByteCount = 0
    stream.extend_from_slice(&bands_data);

    let pixels = dec.decode(&stream, width, height).unwrap();
    assert_eq!(pixels.len(), usize::from(width) * usize::from(height) * 4);

    // Check column 1, row 1: should be red (from short V-bar pixel data)
    let row1_col1 = (usize::from(width) + 1) * 4;
    assert_eq!(pixels[row1_col1], 0x00, "blue channel at (1,1)");
    assert_eq!(pixels[row1_col1 + 1], 0x00, "green channel at (1,1)");
    assert_eq!(pixels[row1_col1 + 2], 0xFF, "red channel at (1,1)");

    // Check column 1, row 0: should be background (black, from band bkg)
    let row0_col1 = 4; // row=0, col=1 -> offset 4
    assert_eq!(pixels[row0_col1], 0x00, "blue channel at (1,0)");
    assert_eq!(pixels[row0_col1 + 1], 0x00, "green channel at (1,0)");
    assert_eq!(pixels[row0_col1 + 2], 0x00, "red channel at (1,0)");

    // Check column 1, row 3: should also be background
    let idx3 = (3 * usize::from(width) + 1) * 4;
    assert_eq!(pixels[idx3], 0x00, "blue channel at (1,3)");
    assert_eq!(pixels[idx3 + 1], 0x00, "green channel at (1,3)");
    assert_eq!(pixels[idx3 + 2], 0x00, "red channel at (1,3)");
}

#[test]
fn adversarial_large_dimensions_rejected() {
    // 65535x65535 would allocate ~17GB. The decoder should reject it.
    let mut dec = ClearCodecDecoder::new();
    let residual = make_solid_residual(0, 0, 0, 1);
    let stream = make_residual_stream(0, 0, None, &residual);
    assert!(dec.decode(&stream, u16::MAX, u16::MAX).is_err());
}

#[test]
fn large_but_reasonable_dimensions_accepted() {
    // 1920x1080 = 2,073,600 pixels should work fine
    let mut dec = ClearCodecDecoder::new();
    let residual = make_solid_residual(0x42, 0x42, 0x42, 1920 * 1080);
    let stream = make_residual_stream(0, 0, None, &residual);
    let result = dec.decode(&stream, 1920, 1080).unwrap();
    assert_eq!(result.len(), 1920 * 1080 * 4);
    // Spot-check first pixel
    assert_eq!(&result[..4], &[0x42, 0x42, 0x42, 0xFF]);
}
