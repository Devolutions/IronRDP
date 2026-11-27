//! Access-unit parsing of H.264 Annex B streams, on NAL units captured from a VideoToolbox
//! Constrained Baseline encoder feeding an AVC420 server.

use ironrdp_egfx::annex_b::{AccessUnit, NalUnits, ParameterSets, SequenceParameterSet};
use ironrdp_egfx::pdu::avc_to_annex_b_into;

/// The capture's sequence parameter set.
const SPS: [u8; 20] = [
    0x27, 0x42, 0xC0, 0x28, 0x89, 0x8A, 0x28, 0x0F, 0x00, 0x44, 0xFC, 0xB3, 0x50, 0x20, 0x20, 0x20, 0x78, 0x40, 0x21,
    0x2C,
];
/// The capture's picture parameter set.
const PPS: [u8; 4] = [0x28, 0xCE, 0x3C, 0x80];
/// The first bytes of the capture's IDR slice (`nal_unit_type` 5).
const IDR: [u8; 12] = [0x25, 0xB8, 0x00, 0x40, 0x00, 0x8D, 0xFF, 0xFF, 0xF8, 0x7A, 0x28, 0x01];
/// The first bytes of one of the capture's non-IDR slices (`nal_unit_type` 1).
const NON_IDR: [u8; 8] = [0x21, 0xE0, 0x02, 0x00, 0x12, 0x56, 0x75, 0x0A];

fn annex_b(start_code: &[u8], units: &[&[u8]]) -> Vec<u8> {
    let mut stream = Vec::new();
    for unit in units {
        stream.extend_from_slice(start_code);
        stream.extend_from_slice(unit);
    }
    stream
}

const FOUR: &[u8] = &[0, 0, 0, 1];
const THREE: &[u8] = &[0, 0, 1];

// oracle: reference VideoToolbox capture (four-byte start codes before SPS, PPS and IDR slice)
#[test]
fn a_captured_key_access_unit_is_self_contained() {
    let data = annex_b(FOUR, &[&SPS, &PPS, &IDR]);

    let unit = AccessUnit::parse(&data);
    let key = ParameterSets::default().key_access_unit(&data);

    assert!(unit.is_idr());
    // oracle: spec ITU-T H.264 §7.3.2.2, §7.3.3 (decoded by hand: PPS 0 refers to SPS 0, the IDR slice to PPS 0)
    assert_eq!(unit.sps.get(&0), Some(&&SPS[..]));
    assert_eq!(unit.pps.get(&0), Some(&&PPS[..]));
    assert_eq!(unit.slice_pps, [0]);
    assert_eq!(key.map(|key| key.data), Some(data));
}

// oracle: spec ITU-T H.264 §B.2 (a start code prefix is 0x000001; a four-byte one has a leading zero_byte)
#[test]
fn three_and_four_byte_start_codes_delimit_the_same_units() {
    let mut mixed = annex_b(THREE, &[&SPS]);
    mixed.extend(annex_b(FOUR, &[&PPS]));
    mixed.extend(annex_b(THREE, &[&IDR]));

    let units: Vec<&[u8]> = NalUnits::new(&mixed).collect();

    assert_eq!(units, [&SPS[..], &PPS[..], &IDR[..]]);
}

// oracle: spec ITU-T H.264 §7.4.1.2.3 (only an IDR picture starts a decodable stream; parameter sets are not pictures)
#[test]
fn parameter_sets_before_a_non_idr_slice_are_not_a_key_frame() {
    let data = annex_b(FOUR, &[&SPS, &PPS, &NON_IDR]);

    let unit = AccessUnit::parse(&data);

    assert!(!unit.is_idr());
    assert_eq!(ParameterSets::default().key_access_unit(&data), None);
}

// oracle: spec ITU-T H.264 §7.4.1.2.3 (a sequence parameter set alone holds no picture)
#[test]
fn a_sequence_parameter_set_alone_is_not_a_key_frame() {
    let data = annex_b(FOUR, &[&SPS]);

    assert!(!AccessUnit::parse(&data).is_idr());
    assert_eq!(ParameterSets::default().key_access_unit(&data), None);
}

// oracle: spec ITU-T H.264 §7.4.1.2.1 (a slice refers to the parameter sets activated before it)
#[test]
fn an_idr_without_parameter_sets_gets_the_latest_ones() {
    let idr_only = annex_b(FOUR, &[&IDR]);
    let mut sets = ParameterSets::default();
    let before_any = sets.key_access_unit(&idr_only);

    sets.update(&AccessUnit::parse(&annex_b(FOUR, &[&SPS, &PPS, &IDR])));
    let key = sets.key_access_unit(&idr_only).expect("parameter sets were seen");

    assert_eq!(before_any, None);
    assert_eq!(key.data, annex_b(FOUR, &[&SPS, &PPS, &IDR]));
    assert_eq!((key.sps.width, key.sps.height), (1920, 1080));
}

// oracle: spec ITU-T H.264 §B.2 (trailing_zero_8bits after a NAL unit belong to the byte stream, not the unit)
#[test]
fn a_truncated_stream_yields_its_units_as_far_as_they_go() {
    let mut data = annex_b(FOUR, &[&SPS, &IDR[..4]]);
    data.extend_from_slice(&[0, 0]);
    data.extend_from_slice(&[0, 0, 1]);

    let units: Vec<&[u8]> = NalUnits::new(&data).collect();

    assert_eq!(units, [&SPS[..], &IDR[..4]]);
}

// oracle: spec ITU-T H.264 §7.4.1 (forbidden_zero_bit shall be 0)
#[test]
fn a_unit_with_the_forbidden_bit_is_ignored() {
    let forbidden = [0x80 | IDR[0], IDR[1]];
    let data = annex_b(FOUR, &[&SPS, &PPS, &forbidden]);

    assert!(!AccessUnit::parse(&data).idr);
}

// oracle: spec ISO/IEC 14496-15 §5.3.3 (an AVC sample is NAL units each behind a big-endian length)
#[test]
fn converted_avc_input_classifies_like_annex_b() {
    let mut avc = Vec::new();
    for unit in [&SPS[..], &PPS[..], &IDR[..]] {
        avc.extend_from_slice(&u32::try_from(unit.len()).unwrap().to_be_bytes());
        avc.extend_from_slice(unit);
    }
    let mut converted = Vec::new();
    avc_to_annex_b_into(&avc, &mut converted);

    assert!(AccessUnit::parse(&converted).is_idr());
    assert!(ParameterSets::default().key_access_unit(&converted).is_some());
}

// oracle: spec ITU-T H.264 §7.3.2.1.1 (fields decoded by hand: profile 66, constraints 0xC0, level 40,
// 120x68 macroblocks, frame_mbs_only, bottom crop 4 units of 2 rows: 1920x1080)
#[test]
fn the_captured_sequence_parameter_set_describes_a_1080p_constrained_baseline_stream() {
    let sps = SequenceParameterSet::parse(&SPS).expect("a valid SPS");

    assert_eq!(
        sps,
        SequenceParameterSet {
            profile_idc: 66,
            constraint_flags: 0xC0,
            level_idc: 40,
            width: 1920,
            height: 1080,
            // oracle: spec ITU-T H.264 §E.1.1 (decoded by hand: VUI with video_signal_type, video_format 5, full range 0)
            full_range: Some(false),
        }
    );
    // oracle: spec RFC 6381 §3.3 (avc1. then profile_idc, the constraint byte and level_idc in hexadecimal)
    assert_eq!(sps.codec_string(), "avc1.42C028");
}

/// The SPS of the same encoder set to full range, as MS-RDPEGFX §3.3.8.3.1 requires.
// oracle: spec ITU-T H.264 §E.1.1 (decoded by hand: the byte after 0xB3 is 0x70, video_format 5, full range 1)
#[test]
fn a_full_range_sequence_parameter_set_says_so() {
    let mut full = SPS;
    full[12..16].copy_from_slice(&[0x70, 0x10, 0x10, 0x10]);

    let sps = SequenceParameterSet::parse(&full).expect("a valid SPS");

    assert_eq!((sps.width, sps.height, sps.full_range), (1920, 1080, Some(true)));
}

// oracle: spec ITU-T H.264 §7.3.2.1.1 (the SPS ends before its picture size can be read)
#[test]
fn a_truncated_sequence_parameter_set_is_rejected() {
    assert_eq!(SequenceParameterSet::parse(&SPS[..8]), None);
}

/// Builds RBSP bits for hand-made NAL units.
#[derive(Default)]
struct Bits {
    bits: Vec<bool>,
}

impl Bits {
    fn bit(mut self, bit: bool) -> Self {
        self.bits.push(bit);
        self
    }

    fn byte(mut self, byte: u8) -> Self {
        for shift in (0..8).rev() {
            self.bits.push((byte >> shift) & 1 == 1);
        }
        self
    }

    /// An unsigned Exp-Golomb code (ITU-T H.264 §9.1).
    fn exp_golomb(mut self, value: u32) -> Self {
        let code = u64::from(value) + 1;
        let width = 64 - code.leading_zeros();
        for _ in 1..width {
            self.bits.push(false);
        }
        for shift in (0..width).rev() {
            self.bits.push((code >> shift) & 1 == 1);
        }
        self
    }

    /// The NAL unit with `header`, its RBSP trailing bits and emulation prevention bytes (§7.4.1).
    fn nal(mut self, header: u8) -> Vec<u8> {
        self.bits.push(true);
        while !self.bits.len().is_multiple_of(8) {
            self.bits.push(false);
        }
        let mut unit = vec![header];
        let mut zeros = 0;
        for chunk in self.bits.chunks(8) {
            let byte = chunk.iter().fold(0u8, |byte, &bit| (byte << 1) | u8::from(bit));
            if 2 <= zeros && byte <= 3 {
                unit.push(3);
                zeros = 0;
            }
            zeros = if byte == 0 { zeros + 1 } else { 0 };
            unit.push(byte);
        }
        unit
    }
}

/// A Baseline SPS with id `id` for a picture of `width_mbs` x `height_mbs` macroblocks, uncropped.
fn sps(id: u32, width_mbs: u32, height_mbs: u32) -> Vec<u8> {
    Bits::default()
        .byte(66)
        .byte(0xC0)
        .byte(30)
        .exp_golomb(id)
        .exp_golomb(0) // log2_max_frame_num_minus4
        .exp_golomb(2) // pic_order_cnt_type
        .exp_golomb(1) // max_num_ref_frames
        .bit(false) // gaps_in_frame_num_value_allowed_flag
        .exp_golomb(width_mbs - 1)
        .exp_golomb(height_mbs - 1)
        .bit(true) // frame_mbs_only_flag
        .bit(true) // direct_8x8_inference_flag
        .bit(false) // frame_cropping_flag
        .bit(false) // vui_parameters_present_flag
        .nal(0x67)
}

/// A PPS with id `id` referring to SPS `sps_id`.
fn pps(id: u32, sps_id: u32) -> Vec<u8> {
    Bits::default()
        .exp_golomb(id)
        .exp_golomb(sps_id)
        .bit(false)
        .bit(false)
        .nal(0x68)
}

/// The start of an IDR I-slice referring to PPS `pps_id`.
fn idr_slice(pps_id: u32) -> Vec<u8> {
    Bits::default()
        .exp_golomb(0)
        .exp_golomb(7)
        .exp_golomb(pps_id)
        .exp_golomb(0)
        .nal(0x65)
}

/// Parameter sets announced under alternating ids are resolved slice to PPS to SPS, so a decoder
/// starting on an IDR gets the sets that IDR refers to, not the last ones seen.
// oracle: spec ITU-T H.264 §7.4.1.2.1 (a slice activates the PPS it names, which activates the SPS it names)
#[test]
fn an_idr_gets_the_parameter_sets_its_slice_refers_to() {
    let (sps_a, sps_b) = (sps(0, 40, 30), sps(1, 80, 45));
    let (pps_a, pps_b) = (pps(0, 0), pps(1, 1));
    let mut sets = ParameterSets::default();
    sets.update(&AccessUnit::parse(&annex_b(FOUR, &[&sps_a, &pps_a, &sps_b, &pps_b])));
    let idr_on_a = annex_b(FOUR, &[&idr_slice(0)]);

    let key = sets.key_access_unit(&idr_on_a).expect("the referenced sets were seen");

    assert_eq!(key.data, annex_b(FOUR, &[&sps_a, &pps_a, &idr_slice(0)]));
    assert_eq!((key.sps.width, key.sps.height), (640, 480));
}

/// A slice naming a PPS never seen gives no key access unit.
// oracle: spec ITU-T H.264 §7.4.3 (pic_parameter_set_id names an available PPS)
#[test]
fn an_idr_referring_to_an_unknown_pps_is_not_a_key() {
    let mut sets = ParameterSets::default();
    sets.update(&AccessUnit::parse(&annex_b(FOUR, &[&sps(0, 40, 30), &pps(0, 0)])));

    assert_eq!(sets.key_access_unit(&annex_b(FOUR, &[&idr_slice(5)])), None);
}

/// Out-of-range identifiers mark the access unit malformed instead of being stored.
// oracle: spec ITU-T H.264 §7.4.2.1.1 (seq_parameter_set_id is in 0..=31)
#[test]
fn a_parameter_set_with_an_out_of_range_id_is_malformed() {
    let unit_data = annex_b(FOUR, &[&sps(32, 40, 30)]);
    let unit = AccessUnit::parse(&unit_data);

    assert!(unit.malformed);
    assert!(unit.sps.is_empty());
}

/// Cropping offsets large enough to overflow the crop arithmetic are rejected, not wrapped.
// oracle: regression codex-web-h264-rereview R1 (frame_crop_left_offset 0x80000000 times CropUnitX 2 panicked)
#[test]
fn an_sps_with_an_overflowing_crop_is_rejected() {
    let unit = [
        0x67, 0x42, 0x00, 0x28, 0xFB, 0xE0, 0x00, 0x00, 0x03, 0x00, 0x20, 0x00, 0x00, 0x03, 0x00, 0x7C,
    ];

    assert_eq!(SequenceParameterSet::parse(&unit), None);
}

/// A crop removing the whole picture is rejected.
// oracle: spec ITU-T H.264 §7.4.2.1.1 (the cropped frame keeps at least one sample)
#[test]
fn an_sps_cropped_to_nothing_is_rejected() {
    let unit = Bits::default()
        .byte(66)
        .byte(0xC0)
        .byte(30)
        .exp_golomb(0)
        .exp_golomb(0)
        .exp_golomb(2)
        .exp_golomb(1)
        .bit(false)
        .exp_golomb(0) // one macroblock wide
        .exp_golomb(0)
        .bit(true)
        .bit(true)
        .bit(true) // frame_cropping_flag
        .exp_golomb(4) // left: 4 x 2 = 8 columns
        .exp_golomb(4) // right: 8 more, the whole 16
        .exp_golomb(0)
        .exp_golomb(0)
        .bit(false)
        .nal(0x67);

    assert_eq!(SequenceParameterSet::parse(&unit), None);
}
