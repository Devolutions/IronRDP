//! Volume scaling unit tests for `ironrdp-rdpsnd-native` (crate has `test = false`).

use ironrdp_rdpsnd_native::cpal::{apply_volume, pack_volume, unpack_volume};

#[test]
fn pack_unpack_roundtrip() {
    let packed = pack_volume(0x1234, 0xABCD);
    assert_eq!(unpack_volume(packed), (0x1234, 0xABCD));
}

#[test]
fn apply_volume_full_scale_is_noop_for_i16() {
    let mut data = i16::MAX.to_le_bytes().to_vec();
    data.extend_from_slice(&i16::MIN.to_le_bytes());
    let original = data.clone();
    let mut phase = 0;
    apply_volume(&mut data, 16, 2, 0xFFFF, 0xFFFF, &mut phase);
    assert_eq!(data, original);
    assert_eq!(phase, 2);
}

#[test]
fn apply_volume_half_scales_i16_stereo_lanes() {
    // Interleaved L,R samples at full scale, half volume on left only.
    let mut data = Vec::new();
    data.extend_from_slice(&10_000i16.to_le_bytes());
    data.extend_from_slice(&10_000i16.to_le_bytes());
    let mut phase = 0;
    apply_volume(&mut data, 16, 2, 0x8000, 0xFFFF, &mut phase);

    let left = i16::from_le_bytes([data[0], data[1]]);
    let right = i16::from_le_bytes([data[2], data[3]]);
    // 10000 * 0x8000 / 0xFFFF ≈ 5000
    assert!((left - 5000).abs() <= 1);
    assert_eq!(right, 10_000);
    assert_eq!(phase, 2);
}

#[test]
fn apply_volume_preserves_phase_across_blocks() {
    // First block ends mid-frame (odd sample count for stereo) — phase must carry.
    let mut first = 8_000i16.to_le_bytes().to_vec(); // only left sample
    let mut phase = 0;
    apply_volume(&mut first, 16, 2, 0x8000, 0x4000, &mut phase);
    assert_eq!(phase, 1);

    let mut second = 8_000i16.to_le_bytes().to_vec(); // should use right volume
    apply_volume(&mut second, 16, 2, 0x8000, 0x4000, &mut phase);
    let right = i16::from_le_bytes([second[0], second[1]]);
    // 8000 * 0x4000 / 0xFFFF ≈ 2000
    assert!((right - 2000).abs() <= 1);
    assert_eq!(phase, 2);
}

#[test]
fn apply_volume_u8_centers_around_128() {
    let mut data = vec![128u8, 200];
    let mut phase = 0;
    apply_volume(&mut data, 8, 1, 0x8000, 0x8000, &mut phase);
    assert_eq!(data[0], 128); // silence stays silence
    // (200-128)*0x8000/0xFFFF + 128 ≈ 164
    assert!((i16::from(data[1]) - 164).abs() <= 1);
}
