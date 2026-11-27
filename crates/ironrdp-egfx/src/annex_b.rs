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
use std::collections::{BTreeMap, BTreeSet};

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
/// The `profile_idc` values whose sequence parameter set carries `chroma_format_idc`, the bit depths
/// and the scaling matrices, in the order of the condition in the SPS syntax (7.3.2.1.1).
const PROFILES_WITH_CHROMA_INFO: [u8; 13] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

const START_CODE: [u8; 4] = [0, 0, 0, 1];
const SPS_BYTES_BEFORE_ID: usize = 3;
const MAX_SLICE_TYPE: u32 = 9;
const MACROBLOCK_SIZE: u32 = 16;

const CHROMA_MONOCHROME: u32 = 0;
const CHROMA_420: u32 = 1;
const CHROMA_422: u32 = 2;
const CHROMA_444: u32 = 3;

const SCALING_LISTS_4X4: usize = 6;

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
    let [id] = leading_codes::<1>(unit, SPS_BYTES_BEFORE_ID)?;
    u8::try_from(id).ok().filter(|_| id <= MAX_SPS_ID)
}

/// `pic_parameter_set_id` and `seq_parameter_set_id` of a picture parameter set.
fn pps_and_sps_ids(unit: &[u8]) -> Option<(u8, u8)> {
    let [pps, sps] = leading_codes::<2>(unit, 0)?;
    if !(pps <= MAX_PPS_ID && sps <= MAX_SPS_ID) {
        return None;
    }
    Some((u8::try_from(pps).ok()?, u8::try_from(sps).ok()?))
}

/// `pic_parameter_set_id` of a slice, after `first_mb_in_slice` and `slice_type`.
fn slice_pps_id(unit: &[u8]) -> Option<u8> {
    let [_, slice_type, pps] = leading_codes::<3>(unit, 0)?;
    if !(slice_type <= MAX_SLICE_TYPE && pps <= MAX_PPS_ID) {
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
                Some(NAL_PPS) => match pps_and_sps_ids(nal) {
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
        let pps_ids: BTreeSet<u8> = unit.slice_pps.iter().copied().collect();
        let sps_id = self.single_sps_id(&unit, &pps_ids)?;
        let sps_unit = self.sps_unit(&unit, sps_id)?;
        let sps = SequenceParameterSet::parse(sps_unit)?;

        let mut missing_sets = Vec::new();
        if !unit.sps.contains_key(&sps_id) {
            missing_sets.push(sps_unit);
        }
        for pps_id in pps_ids {
            if !unit.pps.contains_key(&pps_id) {
                missing_sets.push(self.pps.get(&pps_id)?.as_slice());
            }
        }
        Some(KeyAccessUnit {
            data: prepend_nal_units(&missing_sets, data),
            sps,
        })
    }

    fn sps_unit<'a>(&'a self, unit: &AccessUnit<'a>, id: u8) -> Option<&'a [u8]> {
        unit.sps
            .get(&id)
            .copied()
            .or_else(|| self.sps.get(&id).map(Vec::as_slice))
    }

    fn pps_unit<'a>(&'a self, unit: &AccessUnit<'a>, id: u8) -> Option<&'a [u8]> {
        unit.pps
            .get(&id)
            .copied()
            .or_else(|| self.pps.get(&id).map(Vec::as_slice))
    }

    fn single_sps_id(&self, unit: &AccessUnit<'_>, pps_ids: &BTreeSet<u8>) -> Option<u8> {
        let sps_ids = pps_ids
            .iter()
            .map(|&pps_id| {
                let (_, sps_id) = pps_and_sps_ids(self.pps_unit(unit, pps_id)?)?;
                Some(sps_id)
            })
            .collect::<Option<BTreeSet<u8>>>()?;
        let mut sps_ids = sps_ids.into_iter();
        match (sps_ids.next(), sps_ids.next()) {
            (Some(sps_id), None) => Some(sps_id),
            _ => None,
        }
    }
}

fn prepend_nal_units(units: &[&[u8]], data: &[u8]) -> Vec<u8> {
    let prefix_len: usize = units.iter().map(|unit| START_CODE.len() + unit.len()).sum();
    let mut joined = Vec::with_capacity(prefix_len + data.len());
    for unit in units {
        joined.extend_from_slice(&START_CODE);
        joined.extend_from_slice(unit);
    }
    joined.extend_from_slice(data);
    joined
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
        let mut chroma_format_idc = CHROMA_420;
        if PROFILES_WITH_CHROMA_INFO.contains(&profile_idc) {
            chroma_format_idc = bits.at_most(CHROMA_444)?;
            if chroma_format_idc == CHROMA_444 {
                bits.bit()?; // separate_colour_plane_flag
            }
            bits.at_most(6)?; // bit_depth_luma_minus8
            bits.at_most(6)?; // bit_depth_chroma_minus8
            bits.bit()?; // qpprime_y_zero_transform_bypass_flag
            if bits.bit()? {
                let scaling_lists_8x8 = if chroma_format_idc == CHROMA_444 { 6 } else { 2 };
                for list in 0..SCALING_LISTS_4X4 + scaling_lists_8x8 {
                    if bits.bit()? {
                        skip_scaling_list(&mut bits, if list < SCALING_LISTS_4X4 { 4 * 4 } else { 8 * 8 })?;
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
        let (mut crop_left, mut crop_right, mut crop_top, mut crop_bottom) = (0, 0, 0, 0);
        if bits.bit()? {
            crop_left = bits.exp_golomb()?;
            crop_right = bits.exp_golomb()?;
            crop_top = bits.exp_golomb()?;
            crop_bottom = bits.exp_golomb()?;
        }
        let full_range = video_full_range(&mut bits);
        // 7.4.2.1.1, frame_crop_left_offset: CropUnitX and CropUnitY by ChromaArrayType.
        let field_factor: u32 = if frame_mbs_only { 1 } else { 2 };
        let (crop_unit_x, crop_unit_y): (u32, u32) = match chroma_format_idc {
            CHROMA_MONOCHROME => (1, field_factor),
            CHROMA_420 => (2, 2 * field_factor),
            CHROMA_422 => (2, field_factor),
            _ => (1, field_factor),
        };
        let cropped_width = crop_unit_x.checked_mul(crop_left.checked_add(crop_right)?)?;
        let cropped_height = crop_unit_y.checked_mul(crop_top.checked_add(crop_bottom)?)?;
        let width = (width_in_mbs * MACROBLOCK_SIZE).checked_sub(cropped_width)?;
        let height = (height_in_map_units * MACROBLOCK_SIZE * field_factor).checked_sub(cropped_height)?;
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
