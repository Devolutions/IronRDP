//! Audio Output Virtual Channel Extension PDUs  [MS-RDPEA][1] implementation.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b

use std::borrow::Cow;
use std::fmt;

use bitflags::bitflags;
use ironrdp_core::{
    cast_length, ensure_fixed_part_size, ensure_size, invalid_field_err, other_err, Decode, DecodeError, DecodeResult,
    Encode, EncodeResult, ReadCursor, WriteCursor,
};
use ironrdp_pdu::{read_padding, write_padding};
use ironrdp_svc::SvcEncode;

const SNDC_FORMATS: u8 = 0x07;
const SNDC_QUALITYMODE: u8 = 0x0C;
const SNDC_CRYPTKEY: u8 = 0x08;
const SNDC_TRAINING: u8 = 0x06;
const SNDC_WAVE: u8 = 0x02;
const SNDC_WAVECONFIRM: u8 = 0x05;
const SNDC_WAVEENCRYPT: u8 = 0x09;
const SNDC_CLOSE: u8 = 0x01;
const SNDC_WAVE2: u8 = 0x0D;
const SNDC_VOLUME: u8 = 0x03;
const SNDC_PITCH: u8 = 0x04;

// TODO: UDP PDUs

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq)]
pub enum Version {
    V2 = 0x02,
    V5 = 0x05,
    V6 = 0x06,
    V8 = 0x08,
}

impl TryFrom<u16> for Version {
    type Error = DecodeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x02 => Ok(Self::V2),
            0x05 => Ok(Self::V5),
            0x06 => Ok(Self::V6),
            0x08 => Ok(Self::V8),
            _ => Err(invalid_field_err!("Version", "unknown audio output version")),
        }
    }
}

impl From<Version> for u16 {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn from(version: Version) -> Self {
        version as u16
    }
}

// format tag:
// http://tools.ietf.org/html/rfc2361
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WaveFormat(u16);

macro_rules! wave_formats {
    (
        $(
            ($konst:ident, $num:expr);
        )+
    ) => {
        impl WaveFormat {
        $(
            pub const $konst: WaveFormat = WaveFormat($num);
        )+

            fn as_str(&self) -> Option<&'static str> {
                match self.0 {
                    $(
                        $num => Some(stringify!($konst)),
                    )+
                        _ => None
                }
            }
        }
    }
}

wave_formats! {
    (UNKNOWN, 0x0000);
    (PCM, 0x0001);
    (ADPCM, 0x0002);
    (IEEE_FLOAT, 0x0003);
    (VSELP, 0x0004);
    (IBM_CVSD, 0x0005);
    (ALAW, 0x0006);
    (MULAW, 0x0007);
    (OKI_ADPCM, 0x0010);
    (DVI_ADPCM, 0x0011);
    (MEDIASPACE_ADPCM, 0x0012);
    (SIERRA_ADPCM, 0x0013);
    (G723_ADPCM, 0x0014);
    (DIGISTD, 0x0015);
    (DIGIFIX, 0x0016);
    (DIALOGIC_OKI_ADPCM, 0x0017);
    (MEDIAVISION_ADPCM, 0x0018);
    (CU_CODEC, 0x0019);
    (YAMAHA_ADPCM, 0x0020);
    (SONARC, 0x0021);
    (DSPGROUP_TRUESPEECH, 0x0022);
    (ECHOSC1, 0x0023);
    (AUDIOFILE_AF36, 0x0024);
    (APTX, 0x0025);
    (AUDIOFILE_AF10, 0x0026);
    (PROSODY_1612, 0x0027);
    (LRC, 0x0028);
    (DOLBY_AC2, 0x0030);
    (GSM610, 0x0031);
    (MSNAUDIO, 0x0032);
    (ANTEX_ADPCME, 0x0033);
    (CONTROL_RES_VQLPC, 0x0034);
    (DIGIREAL, 0x0035);
    (DIGIADPCM, 0x0036);
    (CONTROL_RES_CR10, 0x0037);
    (NMS_VBXADPCM, 0x0038);
    (ROLAND_RDAC, 0x0039);
    (ECHOSC3, 0x003A);
    (ROCKWELL_ADPCM, 0x003B);
    (ROCKWELL_DIGITALK, 0x003C);
    (XEBEC, 0x003D);
    (G721_ADPCM, 0x0040);
    (G728_CELP, 0x0041);
    (MSG723, 0x0042);
    (MPEG, 0x0050);
    (RT24, 0x0052);
    (PAC, 0x0053);
    (MPEGLAYER3, 0x0055);
    (LUCENT_G723, 0x0059);
    (CIRRUS, 0x0060);
    (ESPCM, 0x0061);
    (VOXWARE, 0x0062);
    (CANOPUS_ATRAC, 0x0063);
    (G726_ADPCM, 0x0064);
    (G722_ADPCM, 0x0065);
    (DSAT, 0x0066);
    (DSAT_DISPLAY, 0x0067);
    (VOXWARE_BYTE_ALIGNED, 0x0069);
    (VOXWARE_AC8, 0x0070);
    (VOXWARE_AC10, 0x0071);
    (VOXWARE_AC16, 0x0072);
    (VOXWARE_AC20, 0x0073);
    (VOXWARE_RT24, 0x0074);
    (VOXWARE_RT29, 0x0075);
    (VOXWARE_RT29HW, 0x0076);
    (VOXWARE_VR12, 0x0077);
    (VOXWARE_VR18, 0x0078);
    (VOXWARE_TQ40, 0x0079);
    (SOFTSOUND, 0x0080);
    (VOXWARE_TQ60, 0x0081);
    (MSRT24, 0x0082);
    (G729A, 0x0083);
    (MVI_MV12, 0x0084);
    (DF_G726, 0x0085);
    (DF_GSM610, 0x0086);
    (ISIAUDIO, 0x0088);
    (ONLIVE, 0x0089);
    (SBC24, 0x0091);
    (DOLBY_AC3_SPDIF, 0x0092);
    (ZYXEL_ADPCM, 0x0097);
    (PHILIPS_LPCBB, 0x0098);
    (PACKED, 0x0099);
    (RHETOREX_ADPCM, 0x0100);
    (IRAT, 0x0101);
    (VIVO_G723, 0x0111);
    (VIVO_SIREN, 0x0112);
    (DIGITAL_G723, 0x0123);
    (WMAUDIO2, 0x0161);
    (WMAUDIO3, 0x0162);
    (WMAUDIO_LOSSLESS, 0x0163);
    (CREATIVE_ADPCM, 0x0200);
    (CREATIVE_FASTSPEECH8, 0x0202);
    (CREATIVE_FASTSPEECH10, 0x0203);
    (QUARTERDECK, 0x0220);
    (FM_TOWNS_SND, 0x0300);
    (BTV_DIGITAL, 0x0400);
    (VME_VMPCM, 0x0680);
    (OLIGSM, 0x1000);
    (OLIADPCM, 0x1001);
    (OLICELP, 0x1002);
    (OLISBC, 0x1003);
    (OLIOPR, 0x1004);
    (LH_CODEC, 0x1100);
    (NORRIS, 0x1400);
    (SOUNDSPACE_MUSICOMPRESS, 0x1500);
    (DVM, 0x2000);
    (OPUS, 0x704F);
    (AAC_MS, 0xA106);
    (EXTENSIBLE, 0xFFFE);
}

impl fmt::Debug for WaveFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for WaveFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.0, self.as_str().unwrap_or("<unknown wave format>"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    pub format: WaveFormat,
    pub n_channels: u16,
    pub n_samples_per_sec: u32,
    pub n_avg_bytes_per_sec: u32,
    pub n_block_align: u16,
    pub bits_per_sample: u16,
    pub data: Option<Vec<u8>>,
}

impl AudioFormat {
    const NAME: &'static str = "SERVER_AUDIO_VERSION_AND_FORMATS";

    const FIXED_PART_SIZE: usize =
        2 /* wFormatTag */
        + 2 /* nChannels */
        + 4 /* nSamplesPerSec */
        + 4 /* nAvgBytesPerSec */
        + 2 /* nBlockAlign */
        + 2 /* wBitsPerSample */
        + 2 /* cbSize */;
}

impl Encode for AudioFormat {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.format.0);
        dst.write_u16(self.n_channels);
        dst.write_u32(self.n_samples_per_sec);
        dst.write_u32(self.n_avg_bytes_per_sec);
        dst.write_u16(self.n_block_align);
        dst.write_u16(self.bits_per_sample);
        let len = self.data.as_ref().map_or(0, |d| d.len());
        dst.write_u16(cast_length!("AudioFormat::cbSize", len)?);
        if let Some(data) = &self.data {
            dst.write_slice(data);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.data.as_ref().map_or(0, |d| d.len()))
            .expect("never overflow")
    }
}

impl<'de> Decode<'de> for AudioFormat {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let format = WaveFormat(src.read_u16());
        let n_channels = src.read_u16();
        let n_samples_per_sec = src.read_u32();
        let n_avg_bytes_per_sec = src.read_u32();
        let n_block_align = src.read_u16();
        let bits_per_sample = src.read_u16();
        let cb_size = cast_length!("cbSize", src.read_u16())?;

        ensure_size!(in: src, size: cb_size);
        let data = if cb_size > 0 {
            Some(src.read_slice(cb_size).into())
        } else {
            None
        };

        Ok(Self {
            format,
            n_channels,
            n_samples_per_sec,
            n_avg_bytes_per_sec,
            n_block_align,
            bits_per_sample,
            data,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAudioFormatPdu {
    pub version: Version,
    pub formats: Vec<AudioFormat>,
}

impl ServerAudioFormatPdu {
    const NAME: &'static str = "SERVER_AUDIO_VERSION_AND_FORMATS";

    const FIXED_PART_SIZE: usize =
        4 /* dwFlags */
        + 4 /* dwVolume */
        + 4 /* dwPitch */
        + 2 /* wDGramPort */
        + 2 /* wNumberOfFormats */
        + 1 /* cLastBlockConfirmed */
        + 2 /* wVersion */
        + 1 /* bPad */;
}

impl Encode for ServerAudioFormatPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        write_padding!(dst, 4); /* flags */
        write_padding!(dst, 4); /* volume */
        write_padding!(dst, 4); /* pitch */
        write_padding!(dst, 2); /* DGramPort */
        dst.write_u16(cast_length!("AudioFormatPdu::n_formats", self.formats.len())?);
        write_padding!(dst, 1); /* blockNo */
        dst.write_u16(self.version.into());
        write_padding!(dst, 1);
        for fmt in self.formats.iter() {
            fmt.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.formats.iter().map(|fmt| fmt.size()).sum::<usize>())
            .expect("never overflow")
    }
}

impl<'de> Decode<'de> for ServerAudioFormatPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_padding!(src, 4); /* flags */
        read_padding!(src, 4); /* volume */
        read_padding!(src, 4); /* pitch */
        read_padding!(src, 2); /* DGramPort */
        let n_formats = usize::from(src.read_u16());
        read_padding!(src, 1); /* blockNo */
        let version = Version::try_from(src.read_u16())?;
        read_padding!(src, 1);
        let formats = core::iter::repeat_with(|| AudioFormat::decode(src))
            .take(n_formats)
            .collect::<DecodeResult<_>>()?;

        Ok(Self { version, formats })
    }
}

bitflags! {
    /// Represents `dwFlags` field of `CLIENT_AUDIO_VERSION_AND_FORMATS` structure.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct AudioFormatFlags: u32 {
        /// The client is capable of consuming audio data. This flag MUST be set
        /// for audio data to be transferred.
        const ALIVE = 0x0000_0001;
        /// The client is capable of applying a volume change to all the audio
        /// data that is received.
        const VOLUME = 0x0000_0002;
        /// The client is capable of applying a pitch change to all the audio
        /// data that is received.
        const PITCH = 0x0000_00004;
        // The source may set any bits
        const _ = !0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAudioFormatPdu {
    pub version: Version,
    pub flags: AudioFormatFlags,
    pub formats: Vec<AudioFormat>,
    pub volume_left: u16,
    pub volume_right: u16,
    pub pitch: u32,
    pub dgram_port: u16,
}

impl ClientAudioFormatPdu {
    const NAME: &'static str = "CLIENT_AUDIO_VERSION_AND_FORMATS";

    const FIXED_PART_SIZE: usize =
        4 /* dwFlags */
        + 4 /* dwVolume */
        + 4 /* dwPitch */
        + 2 /* wDGramPort */
        + 2 /* wNumberOfFormats */
        + 1 /* cLastBlockConfirmed */
        + 2 /* wVersion */
        + 1 /* bPad */;
}

impl Encode for ClientAudioFormatPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.flags.bits());
        let volume = (u32::from(self.volume_right) << 16) | u32::from(self.volume_left);
        dst.write_u32(volume);
        dst.write_u32(self.pitch);
        dst.write_u16_be(self.dgram_port);
        dst.write_u16(cast_length!("AudioFormatPdu::n_formats", self.formats.len())?);
        dst.write_u8(0); /* blockNo */
        dst.write_u16(self.version.into());
        write_padding!(dst, 1);
        for fmt in self.formats.iter() {
            fmt.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.formats.iter().map(|fmt| fmt.size()).sum::<usize>())
            .expect("never overflow")
    }
}

impl<'de> Decode<'de> for ClientAudioFormatPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = AudioFormatFlags::from_bits_truncate(src.read_u32());
        let volume_left = src.read_u16();
        let volume_right = src.read_u16();
        let pitch = src.read_u32();
        let dgram_port = src.read_u16_be();
        let n_formats = usize::from(src.read_u16());
        let _block_no = src.read_u8();
        let version = Version::try_from(src.read_u16())?;
        read_padding!(src, 1);
        let formats = core::iter::repeat_with(|| AudioFormat::decode(src))
            .take(n_formats)
            .collect::<DecodeResult<_>>()?;

        Ok(Self {
            version,
            flags,
            formats,
            volume_left,
            volume_right,
            pitch,
            dgram_port,
        })
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum QualityMode {
    Dynamic = 0x00,
    Medium = 0x01,
    High = 0x02,
}

impl TryFrom<u16> for QualityMode {
    type Error = DecodeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::Dynamic),
            0x01 => Ok(Self::Medium),
            0x02 => Ok(Self::High),
            _ => Err(invalid_field_err!("QualityMode", "unknown audio quality mode")),
        }
    }
}

impl From<QualityMode> for u16 {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn from(mode: QualityMode) -> Self {
        mode as u16
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityModePdu {
    pub quality_mode: QualityMode,
}

impl QualityModePdu {
    const NAME: &'static str = "AUDIO_QUALITY_MODE";

    const FIXED_PART_SIZE: usize =
        2 /* wQualityMode */
        + 2 /* reserved */;
}

impl Encode for QualityModePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.quality_mode.into());
        write_padding!(dst, 2); /* reserved */

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for QualityModePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let quality_mode = QualityMode::try_from(src.read_u16())?;
        read_padding!(src, 2); /* reserved */

        Ok(Self { quality_mode })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptKeyPdu {
    pub seed: [u8; 32],
}

impl CryptKeyPdu {
    const NAME: &'static str = "SNDCRYPT";

    const FIXED_PART_SIZE: usize =
        4 /* reserved */
        + 32 /* seed */;
}

impl Encode for CryptKeyPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        write_padding!(dst, 4); /* reserved */
        dst.write_array(self.seed);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for CryptKeyPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_padding!(src, 4); /* reserved */
        let seed = src.read_array();

        Ok(Self { seed })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingPdu {
    pub timestamp: u16,
    pub data: Vec<u8>,
}

impl TrainingPdu {
    const NAME: &'static str = "SNDTRAINING";

    const FIXED_PART_SIZE: usize =
        2 /* wTimeStamp */
        + 2 /* wPackSize */;
}

impl Encode for TrainingPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.timestamp);
        let len = if self.data.is_empty() {
            0
        } else {
            self.size() + ServerAudioOutputPdu::FIXED_PART_SIZE
        };
        dst.write_u16(cast_length!("TrainingPdu::wPackSize", len)?);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.data.len())
            .expect("never overflow")
    }
}

impl<'de> Decode<'de> for TrainingPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = src.read_u16();
        let len = usize::from(src.read_u16());
        let data = if len != 0 {
            if len < Self::FIXED_PART_SIZE + ServerAudioOutputPdu::FIXED_PART_SIZE {
                return Err(invalid_field_err!("TrainingPdu::wPackSize", "too small"));
            }
            let len = len - Self::FIXED_PART_SIZE - ServerAudioOutputPdu::FIXED_PART_SIZE;
            ensure_size!(in: src, size: len);
            src.read_slice(len).into()
        } else {
            Vec::new()
        };

        Ok(Self { timestamp, data })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingConfirmPdu {
    pub timestamp: u16,
    pub pack_size: u16,
}

impl TrainingConfirmPdu {
    const NAME: &'static str = "SNDTRAININGCONFIRM";

    const FIXED_PART_SIZE: usize =
        2 /* wTimeStamp */
        + 2 /* wPackSize */;
}

impl Encode for TrainingConfirmPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.timestamp);
        dst.write_u16(self.pack_size);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for TrainingConfirmPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = src.read_u16();
        let pack_size = src.read_u16();

        Ok(Self { timestamp, pack_size })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveInfoPdu {
    pub timestamp: u16,
    pub format_no: u16,
    pub block_no: u8,
    pub data: [u8; 4],
}

impl WaveInfoPdu {
    const NAME: &'static str = "SNDWAVEINFO";

    const FIXED_PART_SIZE: usize =
        2 /* wTimeStamp */
        + 2 /* wFormatNo */
        + 1 /* cBlockNo */
        + 3 /* bPad */
        + 4 /* data */;
}

impl Encode for WaveInfoPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.timestamp);
        dst.write_u16(self.format_no);
        dst.write_u8(self.block_no);
        write_padding!(dst, 3);
        dst.write_array(self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for WaveInfoPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = src.read_u16();
        let format_no = src.read_u16();
        let block_no = src.read_u8();
        read_padding!(src, 3);
        let data = src.read_array();

        Ok(Self {
            timestamp,
            format_no,
            block_no,
            data,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SndWavePdu {
    pub data: Vec<u8>,
}

impl SndWavePdu {
    const NAME: &'static str = "SNDWAVE";

    const FIXED_PART_SIZE: usize = 4 /* bPad */;

    fn decode(src: &mut ReadCursor<'_>, data_len: usize) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_padding!(src, 4);
        ensure_size!(in: src, size: data_len);
        let data = src.read_slice(data_len).into();

        Ok(Self { data })
    }
}

impl Encode for SndWavePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        write_padding!(dst, 4);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.data.len())
            .expect("never overflow")
    }
}

// combines WaveInfoPdu + WavePdu
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavePdu<'a> {
    pub timestamp: u16,
    pub format_no: u16,
    pub block_no: u8,
    pub data: Cow<'a, [u8]>,
}

impl WavePdu<'_> {
    const NAME: &'static str = "WavePdu";

    fn body_size(&self) -> usize {
        (WaveInfoPdu::FIXED_PART_SIZE - 4)
            .checked_add(self.data.len())
            .expect("never overflow")
    }
}

impl Encode for WavePdu<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let info = WaveInfoPdu {
            timestamp: self.timestamp,
            format_no: self.format_no,
            block_no: self.block_no,
            data: self.data[0..4]
                .try_into()
                .map_err(|e| other_err!("invalid data", source: e))?,
        };
        let wave = SndWavePdu {
            data: self.data[4..].into(),
        };
        info.encode(dst)?;
        wave.encode(dst)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        (WaveInfoPdu::FIXED_PART_SIZE + SndWavePdu::FIXED_PART_SIZE - 4)
            .checked_add(self.data.len())
            .expect("never overflow")
    }
}

impl WavePdu<'_> {
    fn decode(src: &mut ReadCursor<'_>, body_size: u16) -> DecodeResult<Self> {
        let info = WaveInfoPdu::decode(src)?;
        let body_size = usize::from(body_size);
        let data_len = body_size
            .checked_sub(info.size())
            .ok_or_else(|| invalid_field_err!("Length", "WaveInfo body_size is too small"))?;
        let wave = SndWavePdu::decode(src, data_len)?;

        let mut data = Vec::with_capacity(wave.size());
        data.extend_from_slice(&info.data);
        data.extend_from_slice(&wave.data);

        Ok(Self {
            timestamp: info.timestamp,
            format_no: info.format_no,
            block_no: info.block_no,
            data: data.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveConfirmPdu {
    pub timestamp: u16,
    pub block_no: u8,
}

impl WaveConfirmPdu {
    const NAME: &'static str = "SNDWAV_CONFIRM";

    const FIXED_PART_SIZE: usize =
        2 /* wTimeStamp */
        + 1 /* cConfirmBlockNo */
        + 1 /* pad */;
}

impl Encode for WaveConfirmPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.timestamp);
        dst.write_u8(self.block_no);
        write_padding!(dst, 1);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for WaveConfirmPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = src.read_u16();
        let block_no = src.read_u8();
        read_padding!(src, 1);

        Ok(Self { timestamp, block_no })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveEncryptPdu {
    pub timestamp: u16,
    pub format_no: u16,
    pub block_no: u8,
    pub signature: Option<[u8; 8]>,
    // TODO: use Cow?
    pub data: Vec<u8>,
}

impl WaveEncryptPdu {
    const NAME: &'static str = "SNDWAVECRYPT";

    const FIXED_PART_SIZE: usize =
        2 /* wTimeStamp */
        + 2 /* wFormatNo */
        + 1 /* cBlockNo */
        + 3 /* bPad */;

    fn decode(src: &mut ReadCursor<'_>, version: Version) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = src.read_u16();
        let format_no = src.read_u16();
        let block_no = src.read_u8();
        read_padding!(src, 3);
        let signature = if version >= Version::V5 {
            ensure_size!(in: src, size: 8);
            Some(src.read_array())
        } else {
            None
        };
        let data = src.read_remaining().into();

        Ok(Self {
            timestamp,
            format_no,
            block_no,
            signature,
            data,
        })
    }
}

impl Encode for WaveEncryptPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.timestamp);
        dst.write_u16(self.format_no);
        dst.write_u8(self.block_no);
        write_padding!(dst, 3);
        if let Some(sig) = self.signature.as_ref() {
            dst.write_slice(sig);
        }
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.signature.map_or(0, |_| 8))
            .expect("never overflow")
            .checked_add(self.data.len())
            .expect("never overflow")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Wave2Pdu<'a> {
    pub timestamp: u16,
    pub format_no: u16,
    pub block_no: u8,
    pub audio_timestamp: u32,
    pub data: Cow<'a, [u8]>,
}

impl fmt::Debug for Wave2Pdu<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Wave2Pdu")
            .field("timestamp", &self.timestamp)
            .field("format_no", &self.format_no)
            .field("block_no", &self.block_no)
            .field("audio_timestamp", &self.audio_timestamp)
            .finish()
    }
}

impl Wave2Pdu<'_> {
    const NAME: &'static str = "SNDWAVE2";

    const FIXED_PART_SIZE: usize =
        2 /* wTimeStamp */
        + 2 /* wFormatNo */
        + 1 /* cBlockNo */
        + 3 /* bPad */
        + 4 /* dwAudioTimestamp */;
}

impl Encode for Wave2Pdu<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.timestamp);
        dst.write_u16(self.format_no);
        dst.write_u8(self.block_no);
        write_padding!(dst, 3);
        dst.write_u32(self.audio_timestamp);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.data.len())
            .expect("never overflow")
    }
}

impl<'de> Decode<'de> for Wave2Pdu<'_> {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = src.read_u16();
        let format_no = src.read_u16();
        let block_no = src.read_u8();
        read_padding!(src, 3);
        let audio_timestamp = src.read_u32();
        let data = src.read_remaining().to_vec().into();

        Ok(Self {
            timestamp,
            format_no,
            block_no,
            audio_timestamp,
            data,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumePdu {
    pub volume_left: u16,
    pub volume_right: u16,
}

impl VolumePdu {
    const NAME: &'static str = "SNDVOL";

    const FIXED_PART_SIZE: usize = 4;
}

impl Encode for VolumePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let volume = (u32::from(self.volume_right) << 16) | u32::from(self.volume_left);
        dst.write_u32(volume);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for VolumePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let volume_left = src.read_u16();
        let volume_right = src.read_u16();

        Ok(Self {
            volume_left,
            volume_right,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PitchPdu {
    pub pitch: u32,
}

impl PitchPdu {
    const NAME: &'static str = "SNDPITCH";

    const FIXED_PART_SIZE: usize = 4;
}

impl Encode for PitchPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.pitch);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for PitchPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pitch = src.read_u32();

        Ok(Self { pitch })
    }
}

/// Server Audio Output Channel message (PDU prefixed with `SNDPROLOG`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerAudioOutputPdu<'a> {
    AudioFormat(ServerAudioFormatPdu),
    CryptKey(CryptKeyPdu),
    Training(TrainingPdu),
    Wave(WavePdu<'a>),
    WaveEncrypt(WaveEncryptPdu),
    Close,
    Wave2(Wave2Pdu<'a>),
    Volume(VolumePdu),
    Pitch(PitchPdu),
}

impl ServerAudioOutputPdu<'_> {
    const NAME: &'static str = "ServerAudioOutputPdu";

    const FIXED_PART_SIZE: usize = 1 /* msgType */ + 1 /* padding*/ + 2 /* bodySize */;
}

impl Encode for ServerAudioOutputPdu<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        let (msg_type, pdu_size) = match self {
            Self::AudioFormat(pdu) => (SNDC_FORMATS, pdu.size()),
            Self::CryptKey(pdu) => (SNDC_CRYPTKEY, pdu.size()),
            Self::Training(pdu) => (SNDC_TRAINING, pdu.size()),
            Self::Wave(pdu) => (SNDC_WAVE, pdu.body_size()),
            Self::WaveEncrypt(pdu) => (SNDC_WAVEENCRYPT, pdu.size()),
            Self::Close => (SNDC_CLOSE, 0),
            Self::Wave2(pdu) => (SNDC_WAVE2, pdu.size()),
            Self::Volume(pdu) => (SNDC_VOLUME, pdu.size()),
            Self::Pitch(pdu) => (SNDC_PITCH, pdu.size()),
        };

        dst.write_u8(msg_type);
        write_padding!(dst, 1);
        dst.write_u16(cast_length!("ServerAudioOutputPdu::bodySize", pdu_size)?);

        match self {
            Self::AudioFormat(pdu) => pdu.encode(dst),
            Self::CryptKey(pdu) => pdu.encode(dst),
            Self::Training(pdu) => pdu.encode(dst),
            Self::Wave(pdu) => pdu.encode(dst),
            Self::WaveEncrypt(pdu) => pdu.encode(dst),
            Self::Close => Ok(()),
            Self::Wave2(pdu) => pdu.encode(dst),
            Self::Volume(pdu) => pdu.encode(dst),
            Self::Pitch(pdu) => pdu.encode(dst),
        }?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(match self {
                Self::AudioFormat(pdu) => pdu.size(),
                Self::CryptKey(pdu) => pdu.size(),
                Self::Training(pdu) => pdu.size(),
                Self::Wave(pdu) => pdu.size(),
                Self::WaveEncrypt(pdu) => pdu.size(),
                Self::Close => 0,
                Self::Wave2(pdu) => pdu.size(),
                Self::Volume(pdu) => pdu.size(),
                Self::Pitch(pdu) => pdu.size(),
            })
            .expect("never overflow")
    }
}

impl<'de> Decode<'de> for ServerAudioOutputPdu<'_> {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let msg_type = src.read_u8();
        read_padding!(src, 1);
        let body_size = src.read_u16();

        match msg_type {
            SNDC_FORMATS => {
                let pdu = ServerAudioFormatPdu::decode(src)?;
                Ok(Self::AudioFormat(pdu))
            }
            SNDC_CRYPTKEY => {
                let pdu = CryptKeyPdu::decode(src)?;
                Ok(Self::CryptKey(pdu))
            }
            SNDC_TRAINING => {
                let pdu = TrainingPdu::decode(src)?;
                Ok(Self::Training(pdu))
            }
            SNDC_WAVE => {
                let pdu = WavePdu::decode(src, body_size)?;
                Ok(Self::Wave(pdu))
            }
            SNDC_WAVEENCRYPT => {
                let pdu = WaveEncryptPdu::decode(src, Version::V5)?;
                Ok(Self::WaveEncrypt(pdu))
            }
            SNDC_CLOSE => Ok(Self::Close),
            SNDC_WAVE2 => {
                let pdu = Wave2Pdu::decode(src)?;
                Ok(Self::Wave2(pdu))
            }
            SNDC_VOLUME => {
                let pdu = VolumePdu::decode(src)?;
                Ok(Self::Volume(pdu))
            }
            SNDC_PITCH => {
                let pdu = PitchPdu::decode(src)?;
                Ok(Self::Pitch(pdu))
            }
            _ => Err(invalid_field_err!(
                "ServerAudioOutputPdu::msgType",
                "Unknown audio output PDU type"
            )),
        }
    }
}

impl SvcEncode for ServerAudioOutputPdu<'_> {}

/// Client Audio Output Channel message (PDU prefixed with `SNDPROLOG`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAudioOutputPdu {
    AudioFormat(ClientAudioFormatPdu),
    QualityMode(QualityModePdu),
    TrainingConfirm(TrainingConfirmPdu),
    WaveConfirm(WaveConfirmPdu),
}

impl ClientAudioOutputPdu {
    const NAME: &'static str = "ClientAudioOutputPdu";

    const FIXED_PART_SIZE: usize = 1 /* msgType */ + 1 /* padding*/ + 2 /* bodySize */;
}

impl Encode for ClientAudioOutputPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        let (msg_type, body_size) = match self {
            Self::AudioFormat(pdu) => (SNDC_FORMATS, pdu.size()),
            Self::QualityMode(pdu) => (SNDC_QUALITYMODE, pdu.size()),
            Self::TrainingConfirm(pdu) => (SNDC_TRAINING, pdu.size()),
            Self::WaveConfirm(pdu) => (SNDC_WAVECONFIRM, pdu.size()),
        };

        dst.write_u8(msg_type);
        write_padding!(dst, 1);
        dst.write_u16(cast_length!("ClientAudioOutputPdu::bodySize", body_size)?);

        match self {
            Self::AudioFormat(pdu) => pdu.encode(dst),
            Self::QualityMode(pdu) => pdu.encode(dst),
            Self::TrainingConfirm(pdu) => pdu.encode(dst),
            Self::WaveConfirm(pdu) => pdu.encode(dst),
        }?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(match self {
                Self::AudioFormat(pdu) => pdu.size(),
                Self::QualityMode(pdu) => pdu.size(),
                Self::TrainingConfirm(pdu) => pdu.size(),
                Self::WaveConfirm(pdu) => pdu.size(),
            })
            .expect("never overflow")
    }
}

impl<'de> Decode<'de> for ClientAudioOutputPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let msg_type = src.read_u8();
        read_padding!(src, 1);
        let _body_size = src.read_u16();

        match msg_type {
            SNDC_FORMATS => {
                let pdu = ClientAudioFormatPdu::decode(src)?;
                Ok(Self::AudioFormat(pdu))
            }
            SNDC_QUALITYMODE => {
                let pdu = QualityModePdu::decode(src)?;
                Ok(Self::QualityMode(pdu))
            }
            SNDC_TRAINING => {
                let pdu = TrainingConfirmPdu::decode(src)?;
                Ok(Self::TrainingConfirm(pdu))
            }
            SNDC_WAVECONFIRM => {
                let pdu = WaveConfirmPdu::decode(src)?;
                Ok(Self::WaveConfirm(pdu))
            }
            _ => Err(invalid_field_err!(
                "ClientAudioOutputPdu::msgType",
                "Unknown audio output PDU type"
            )),
        }
    }
}

impl SvcEncode for ClientAudioOutputPdu {}
