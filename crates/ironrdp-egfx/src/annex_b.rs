//! Access units of an H.264 Annex B byte stream, as `RFX_AVC420_BITMAP_STREAM` carries them.
//!
//! A decoder that starts on a new stream, or recovers from an error, needs an access unit with an
//! IDR picture and the sequence and picture parameter sets its slices refer to.
//! This module finds those NAL units ([ITU-T H.264] 7.3.1, 7.4.1.2.3, Annex B), keeps the parameter
//! sets by id, resolves the chain from a slice to its picture and sequence parameter sets, and reads
//! the profile, level and picture size of a sequence parameter set ([ITU-T H.264] 7.3.2.1.1).
//! Everything here reads untrusted server data: malformed input yields `None`, never a panic.
//!
//! [ITU-T H.264]: https://www.itu.int/rec/T-REC-H.264

use core::fmt::Write as _;
use std::collections::BTreeMap;

/// `nal_unit_type` of a coded slice of a non-IDR picture.
const NAL_SLICE: u8 = 1;
/// `nal_unit_type` of a coded slice of an IDR picture.
const NAL_IDR_SLICE: u8 = 5;
/// `nal_unit_type` of a sequence parameter set.
const NAL_SPS: u8 = 7;
/// `nal_unit_type` of a picture parameter set.
const NAL_PPS: u8 = 8;

/// `seq_parameter_set_id` is in 0..=31 (7.4.2.1.1).
const MAX_SPS_ID: u32 = 31;
/// `pic_parameter_set_id` is in 0..=255 (7.4.2.2).
const MAX_PPS_ID: u32 = 255;
/// Upper bound on the size of a kept parameter set; real ones are tens of bytes.
const MAX_PARAMETER_SET_BYTES: usize = 4096;
/// Upper bound on a picture side in macroblocks: 16384 pixels, twice the largest RDP desktop.
const MAX_SIDE_IN_MBS: u32 = 1024;

/// The NAL units of an Annex B byte stream, each from its header byte, without start codes.
///
/// Three- and four-byte start codes are accepted, the zero bytes trailing a NAL unit are left out,
/// and a NAL unit cut short by the end of the data is returned as far as it goes.
#[derive(Debug, Clone)]
pub struct NalUnits<'a> {
    rest: &'a [u8],
}

impl<'a> NalUnits<'a> {
    /// The NAL units of `data`, which starts with a start code; bytes before the first one are skipped.
    pub fn new(data: &'a [u8]) -> Self {
        let rest = match find_start_code(data) {
            Some((_, after)) => &data[after..],
            None => &[],
        };
        Self { rest }
    }
}

impl<'a> Iterator for NalUnits<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        loop {
            if self.rest.is_empty() {
                return None;
            }
            let (unit, rest) = match find_start_code(self.rest) {
                Some((at, after)) => (&self.rest[..at], &self.rest[after..]),
                None => (self.rest, &[][..]),
            };
            self.rest = rest;
            let end = unit.iter().rposition(|&byte| byte != 0).map_or(0, |last| last + 1);
            if end != 0 {
                return Some(&unit[..end]);
            }
        }
    }
}

/// The offset of the first `00 00 01` in `data`, and the offset just past it.
fn find_start_code(data: &[u8]) -> Option<(usize, usize)> {
    data.windows(3)
        .position(|window| window == [0, 0, 1])
        .map(|at| (at, at + 3))
}

/// `nal_unit_type` of a NAL unit, or `None` for an empty one or one with `forbidden_zero_bit` set.
pub fn nal_unit_type(unit: &[u8]) -> Option<u8> {
    let header = *unit.first()?;
    (header & 0x80 == 0).then_some(header & 0x1F)
}

/// The leading identifiers of a NAL unit's RBSP: the Exp-Golomb codes after the header byte and
/// `skip_bytes` fixed bytes.
fn leading_codes<const N: usize>(unit: &[u8], skip_bytes: usize) -> Option<[u32; N]> {
    let rbsp = unescape(unit.get(1..)?);
    let mut bits = BitReader::new(rbsp.get(skip_bytes..)?);
    let mut codes = [0; N];
    for code in &mut codes {
        *code = bits.exp_golomb()?;
    }
    Some(codes)
}

/// `seq_parameter_set_id` of a sequence parameter set.
fn sps_id(unit: &[u8]) -> Option<u8> {
    let [id] = leading_codes::<1>(unit, 3)?;
    u8::try_from(id).ok().filter(|_| id <= MAX_SPS_ID)
}

/// `pic_parameter_set_id` and `seq_parameter_set_id` of a picture parameter set.
fn pps_ids(unit: &[u8]) -> Option<(u8, u8)> {
    let [pps, sps] = leading_codes::<2>(unit, 0)?;
    if !(pps <= MAX_PPS_ID && sps <= MAX_SPS_ID) {
        return None;
    }
    Some((u8::try_from(pps).ok()?, u8::try_from(sps).ok()?))
}

/// `pic_parameter_set_id` of a slice, after `first_mb_in_slice` and `slice_type`.
fn slice_pps_id(unit: &[u8]) -> Option<u8> {
    let [_, slice_type, pps] = leading_codes::<3>(unit, 0)?;
    if !(slice_type <= 9 && pps <= MAX_PPS_ID) {
        return None;
    }
    u8::try_from(pps).ok()
}

/// What an access unit holds, as a decoder starting on it needs to know.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessUnit<'a> {
    /// A slice of an IDR picture, which decodes without earlier pictures.
    pub idr: bool,
    /// A slice of a non-IDR picture, which refers to earlier pictures.
    pub non_idr: bool,
    /// A NAL unit that could not be read: a slice or parameter set with invalid identifiers.
    pub malformed: bool,
    /// The sequence parameter sets, by `seq_parameter_set_id`, from their header byte.
    pub sps: BTreeMap<u8, &'a [u8]>,
    /// The picture parameter sets, by `pic_parameter_set_id`, from their header byte.
    pub pps: BTreeMap<u8, &'a [u8]>,
    /// The `pic_parameter_set_id` each slice refers to.
    pub slice_pps: Vec<u8>,
}

impl<'a> AccessUnit<'a> {
    /// Classifies the NAL units of the Annex B access unit `data`.
    pub fn parse(data: &'a [u8]) -> Self {
        let mut unit = Self::default();
        for nal in NalUnits::new(data) {
            match nal_unit_type(nal) {
                Some(kind @ (NAL_IDR_SLICE | NAL_SLICE)) => {
                    if kind == NAL_IDR_SLICE {
                        unit.idr = true;
                    } else {
                        unit.non_idr = true;
                    }
                    match slice_pps_id(nal) {
                        Some(id) => unit.slice_pps.push(id),
                        None => unit.malformed = true,
                    }
                }
                Some(NAL_SPS) => match sps_id(nal) {
                    Some(id) if nal.len() <= MAX_PARAMETER_SET_BYTES => {
                        unit.sps.insert(id, nal);
                    }
                    _ => unit.malformed = true,
                },
                Some(NAL_PPS) => match pps_ids(nal) {
                    Some((id, _)) if nal.len() <= MAX_PARAMETER_SET_BYTES => {
                        unit.pps.insert(id, nal);
                    }
                    _ => unit.malformed = true,
                },
                _ => {}
            }
        }
        unit
    }

    /// Whether this access unit holds an IDR picture and no other picture's slice.
    pub fn is_idr(&self) -> bool {
        self.idr && !self.non_idr && !self.malformed
    }
}

/// An IDR access unit made self-contained, with the sequence parameter set its slices use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyAccessUnit {
    /// The access unit, behind the parameter sets it refers to and does not carry.
    pub data: Vec<u8>,
    /// The sequence parameter set the slices use.
    pub sps: SequenceParameterSet,
}

/// The parameter sets of a stream by id, for a decoder that starts on a later IDR picture.
///
/// At most 32 sequence and 256 picture parameter sets of at most 4 KiB each are kept, as many as
/// their identifiers allow.
#[derive(Debug, Clone, Default)]
pub struct ParameterSets {
    sps: BTreeMap<u8, Vec<u8>>,
    pps: BTreeMap<u8, Vec<u8>>,
}

impl ParameterSets {
    /// Keeps the parameter sets `unit` carries, replacing earlier ones with the same id.
    pub fn update(&mut self, unit: &AccessUnit<'_>) {
        for (&id, &sps) in &unit.sps {
            self.sps.insert(id, sps.to_vec());
        }
        for (&id, &pps) in &unit.pps {
            self.pps.insert(id, pps.to_vec());
        }
    }

    /// Forgets every parameter set, for a new stream.
    pub fn clear(&mut self) {
        self.sps.clear();
        self.pps.clear();
    }

    /// The IDR access unit `data` as a decoder starting on it needs it: behind the parameter sets
    /// its slices refer to that it does not carry, which [`Self::update`] must have seen.
    ///
    /// Returns `None` for an access unit that is not an IDR picture, whose slices refer to
    /// parameter sets never seen or to different sequence parameter sets, or whose sequence
    /// parameter set is invalid.
    pub fn key_access_unit(&self, data: &[u8]) -> Option<KeyAccessUnit> {
        let unit = AccessUnit::parse(data);
        if !unit.is_idr() || unit.slice_pps.is_empty() {
            return None;
        }
        let mut referenced: Vec<u8> = unit.slice_pps.clone();
        referenced.sort_unstable();
        referenced.dedup();
        let mut sps_id = None;
        for &pps_id in &referenced {
            let pps = unit
                .pps
                .get(&pps_id)
                .copied()
                .or_else(|| self.pps.get(&pps_id).map(Vec::as_slice))?;
            let (_, refers_to) = pps_ids(pps)?;
            if *sps_id.get_or_insert(refers_to) != refers_to {
                return None;
            }
        }
        let sps_id = sps_id?;
        let sps_unit = unit
            .sps
            .get(&sps_id)
            .copied()
            .or_else(|| self.sps.get(&sps_id).map(Vec::as_slice))?;
        let sps = SequenceParameterSet::parse(sps_unit)?;

        let mut prefix: Vec<&[u8]> = Vec::new();
        if !unit.sps.contains_key(&sps_id) {
            prefix.push(sps_unit);
        }
        for pps_id in referenced {
            if !unit.pps.contains_key(&pps_id) {
                prefix.push(self.pps.get(&pps_id)?);
            }
        }
        let mut key = Vec::with_capacity(data.len() + prefix.iter().map(|set| set.len() + 4).sum::<usize>());
        for set in prefix {
            key.extend_from_slice(&[0, 0, 0, 1]);
            key.extend_from_slice(set);
        }
        key.extend_from_slice(data);
        Some(KeyAccessUnit { data: key, sps })
    }
}

/// The fields of a sequence parameter set a decoder is configured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SequenceParameterSet {
    /// `profile_idc`.
    pub profile_idc: u8,
    /// The byte of `constraint_set0_flag` to `constraint_set5_flag` and the two reserved bits.
    pub constraint_flags: u8,
    /// `level_idc`.
    pub level_idc: u8,
    /// Picture width in pixels, after the frame cropping; never zero.
    pub width: u32,
    /// Picture height in pixels, after the frame cropping; never zero.
    pub height: u32,
    /// `video_full_range_flag` of the VUI, or `None` when the SPS does not signal it (E.1.1).
    pub full_range: Option<bool>,
}

impl SequenceParameterSet {
    /// Reads the sequence parameter set NAL unit `unit`, from its header byte.
    ///
    /// Returns `None` when it is not a sequence parameter set, is cut short, has a syntax value out
    /// of its range, or crops its picture to nothing.
    pub fn parse(unit: &[u8]) -> Option<Self> {
        if nal_unit_type(unit)? != NAL_SPS {
            return None;
        }
        let rbsp = unescape(&unit[1..]);
        let mut bits = BitReader::new(&rbsp);
        let profile_idc = bits.byte()?;
        let constraint_flags = bits.byte()?;
        let level_idc = bits.byte()?;
        bits.at_most(MAX_SPS_ID)?; // seq_parameter_set_id
        let mut chroma_format_idc = 1;
        if matches!(
            profile_idc,
            100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
        ) {
            chroma_format_idc = bits.at_most(3)?;
            if chroma_format_idc == 3 {
                bits.bit()?; // separate_colour_plane_flag
            }
            bits.at_most(6)?; // bit_depth_luma_minus8
            bits.at_most(6)?; // bit_depth_chroma_minus8
            bits.bit()?; // qpprime_y_zero_transform_bypass_flag
            if bits.bit()? {
                let lists = if chroma_format_idc == 3 { 12 } else { 8 };
                for list in 0..lists {
                    if bits.bit()? {
                        skip_scaling_list(&mut bits, if list < 6 { 16 } else { 64 })?;
                    }
                }
            }
        }
        bits.at_most(12)?; // log2_max_frame_num_minus4
        match bits.at_most(2)? {
            0 => {
                bits.at_most(12)?; // log2_max_pic_order_cnt_lsb_minus4
            }
            1 => {
                bits.bit()?; // delta_pic_order_always_zero_flag
                bits.signed_exp_golomb()?; // offset_for_non_ref_pic
                bits.signed_exp_golomb()?; // offset_for_top_to_bottom_field
                for _ in 0..bits.at_most(255)? {
                    bits.signed_exp_golomb()?; // offset_for_ref_frame
                }
            }
            _ => {}
        }
        bits.at_most(16)?; // max_num_ref_frames
        bits.bit()?; // gaps_in_frame_num_value_allowed_flag
        let width_in_mbs = bits.at_most(MAX_SIDE_IN_MBS - 1)? + 1;
        let height_in_map_units = bits.at_most(MAX_SIDE_IN_MBS - 1)? + 1;
        let frame_mbs_only = bits.bit()?;
        if !frame_mbs_only {
            bits.bit()?; // mb_adaptive_frame_field_flag
        }
        bits.bit()?; // direct_8x8_inference_flag
        let (mut left, mut right, mut top, mut bottom) = (0, 0, 0, 0);
        if bits.bit()? {
            left = bits.exp_golomb()?;
            right = bits.exp_golomb()?;
            top = bits.exp_golomb()?;
            bottom = bits.exp_golomb()?;
        }
        let full_range = video_full_range(&mut bits);
        // 7.4.2.1.1, frame_crop_left_offset: CropUnitX and CropUnitY by ChromaArrayType.
        let field_factor: u32 = if frame_mbs_only { 1 } else { 2 };
        let (crop_x, crop_y): (u32, u32) = match chroma_format_idc {
            0 => (1, field_factor),
            1 => (2, 2 * field_factor),
            2 => (2, field_factor),
            _ => (1, field_factor),
        };
        let width = (width_in_mbs * 16).checked_sub(crop_x.checked_mul(left.checked_add(right)?)?)?;
        let height =
            (height_in_map_units * 16 * field_factor).checked_sub(crop_y.checked_mul(top.checked_add(bottom)?)?)?;
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            profile_idc,
            constraint_flags,
            level_idc,
            width,
            height,
            full_range,
        })
    }

    /// The codec string of this stream for WebCodecs and RFC 6381: `avc1.PPCCLL` in hexadecimal.
    pub fn codec_string(&self) -> String {
        let mut codec = String::from("avc1.");
        let _ = write!(
            codec,
            "{:02X}{:02X}{:02X}",
            self.profile_idc, self.constraint_flags, self.level_idc
        );
        codec
    }
}

/// `video_full_range_flag` from the VUI parameters that start at `bits` (E.1.1), when present.
fn video_full_range(bits: &mut BitReader<'_>) -> Option<bool> {
    if !bits.bit()? {
        return None; // vui_parameters_present_flag
    }
    if bits.bit()? {
        // aspect_ratio_info_present_flag
        if bits.byte()? == 255 {
            bits.bits(32)?; // Extended_SAR: sar_width and sar_height
        }
    }
    if bits.bit()? {
        bits.bit()?; // overscan_appropriate_flag
    }
    if !bits.bit()? {
        return None; // video_signal_type_present_flag
    }
    bits.bits(3)?; // video_format
    bits.bit()
}

fn skip_scaling_list(bits: &mut BitReader<'_>, size: usize) -> Option<()> {
    let (mut last, mut next) = (8i64, 8i64);
    for _ in 0..size {
        if next != 0 {
            let delta = bits.signed_exp_golomb()?;
            if !(-128..=127).contains(&delta) {
                return None;
            }
            next = (last + delta + 256) % 256;
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

/// The RBSP of a NAL unit payload: each `00 00 03` loses its emulation prevention byte.
fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &byte in payload {
        if 2 <= zeros && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        rbsp.push(byte);
    }
    rbsp
}

struct BitReader<'a> {
    data: &'a [u8],
    /// INVARIANT: `position <= data.len() * 8`.
    position: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn bit(&mut self) -> Option<bool> {
        let byte = *self.data.get(self.position / 8)?;
        let bit = (byte >> (7 - self.position % 8)) & 1;
        self.position += 1;
        Some(bit == 1)
    }

    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.bit()?);
        }
        Some(value)
    }

    fn byte(&mut self) -> Option<u8> {
        u8::try_from(self.bits(8)?).ok()
    }

    /// An unsigned Exp-Golomb code (9.1), up to 31 leading zeros.
    fn exp_golomb(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while !self.bit()? {
            zeros += 1;
            if 31 < zeros {
                return None;
            }
        }
        let suffix = self.bits(zeros)?;
        (1u32 << zeros).checked_sub(1)?.checked_add(suffix)
    }

    /// An unsigned Exp-Golomb code no larger than `max`.
    fn at_most(&mut self, max: u32) -> Option<u32> {
        self.exp_golomb().filter(|&value| value <= max)
    }

    /// A signed Exp-Golomb code (9.1.1).
    fn signed_exp_golomb(&mut self) -> Option<i64> {
        let code = i64::from(self.exp_golomb()?);
        Some(if code % 2 == 1 { (code + 1) / 2 } else { -(code / 2) })
    }
}
