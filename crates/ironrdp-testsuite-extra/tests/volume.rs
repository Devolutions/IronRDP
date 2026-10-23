//! Volume scaling unit tests for `ironrdp-rdpsnd-native` (crate has `test = false`).

use ironrdp_rdpsnd_native::cpal::{apply_gain, pack_volume, split_volume, unpack_volume};

// oracle: property round-trip
#[test]
fn pack_unpack_roundtrip() {
    let packed = pack_volume(0x1234, 0xABCD);
    assert_eq!(unpack_volume(packed), (0x1234, 0xABCD));
}

// oracle: mutant unity gain altering samples
#[test]
fn full_gain_is_noop() {
    let mut samples = [i16::MAX, i16::MIN];

    apply_gain(&mut samples, 2, 0xFFFF, 0xFFFF);

    assert_eq!(samples, [i16::MAX, i16::MIN]);
}

// oracle: mutant left and right gains swapped or not applied
#[test]
fn gain_scales_each_stereo_lane() {
    let mut samples = [10_000, 10_000, -8_000, -8_000];

    apply_gain(&mut samples, 2, 0x8000, 0x4000);

    // 10000 * 0x8000 / 0xFFFF ≈ 5000 and 10000 * 0x4000 / 0xFFFF ≈ 2500.
    assert!((samples[0] - 5_000).abs() <= 1);
    assert!((samples[1] - 2_500).abs() <= 1);
    assert!((samples[2] + 4_000).abs() <= 1);
    assert!((samples[3] + 2_000).abs() <= 1);
}

// oracle: mutant mono stream scaled with the right gain
#[test]
fn mono_uses_left_gain() {
    let mut samples = [8_000, 8_000];

    apply_gain(&mut samples, 1, 0x4000, 0xFFFF);

    assert!(samples.iter().all(|sample| (sample - 2_000).abs() <= 1));
}

// oracle: mutant session level or balance taken from the wrong channel
#[test]
fn split_volume_takes_the_louder_channel_as_master() {
    let (master, left, right) = split_volume(0x8000, 0x4000);

    assert!((master - 0.5).abs() < 0.001);
    assert_eq!(left, 0xFFFF);
    assert!((i32::from(right) - 0x7FFF).abs() <= 1);
}

// oracle: mutant division by zero on silence
#[test]
fn split_volume_of_silence() {
    assert_eq!(split_volume(0, 0), (0.0, 0xFFFF, 0xFFFF));
}

// oracle: mutant full volume not mapped to 1.0
#[test]
fn split_volume_of_full_scale() {
    assert_eq!(split_volume(0xFFFF, 0xFFFF), (1.0, 0xFFFF, 0xFFFF));
}
