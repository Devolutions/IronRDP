//! Audio Input Redirection Virtual Channel Extension PDUs [MS-RDPEAI][1].
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeai/2eb8be0c-4f17-418b-9911-edb8d2ffcde5

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};

/// Upper bound on `FramesPerPacket` accepted from the server (~1 s at 48 kHz).
pub const MAX_FRAMES_PER_PACKET: u32 = 48_000;

/// Upper bound on a single capture Data PDU payload (~1 s stereo 16-bit 48 kHz).
pub const MAX_DATA_PACKET_SIZE: usize = 192_000;

/// `MSG_SNDIN_VERSION` (0x01).
pub const MSG_SNDIN_VERSION: u8 = 0x01;
/// `MSG_SNDIN_FORMATS` (0x02).
pub const MSG_SNDIN_FORMATS: u8 = 0x02;
/// `MSG_SNDIN_OPEN` (0x03).
pub const MSG_SNDIN_OPEN: u8 = 0x03;
/// `MSG_SNDIN_OPEN_REPLY` (0x04).
pub const MSG_SNDIN_OPEN_REPLY: u8 = 0x04;
/// `MSG_SNDIN_DATA_INCOMING` (0x05).
pub const MSG_SNDIN_DATA_INCOMING: u8 = 0x05;
/// `MSG_SNDIN_DATA` (0x06).
pub const MSG_SNDIN_DATA: u8 = 0x06;
/// `MSG_SNDIN_FORMATCHANGE` (0x07).
pub const MSG_SNDIN_FORMATCHANGE: u8 = 0x07;

/// Protocol version values from MS-RDPEAI §2.2.2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum Version {
    V1 = 1,
    V2 = 2,
}

impl TryFrom<u32> for Version {
    type Error = ironrdp_core::DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            _ => Err(invalid_field_err!("Version", "unknown audio input protocol version")),
        }
    }
}

impl From<Version> for u32 {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn from(version: Version) -> Self {
        version as u32
    }
}

/// All client/server AUDIO_INPUT PDUs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RdpeaiPdu {
    Version(VersionPdu),
    Formats(FormatsPdu),
    Open(OpenPdu),
    OpenReply(OpenReplyPdu),
    DataIncoming,
    Data(DataPdu),
    FormatChange(FormatChangePdu),
}

impl RdpeaiPdu {
    const NAME: &'static str = "SNDIN_PDU";
}

impl Encode for RdpeaiPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::Version(p) => p.encode(dst),
            Self::Formats(p) => p.encode(dst),
            Self::Open(p) => p.encode(dst),
            Self::OpenReply(p) => p.encode(dst),
            Self::DataIncoming => {
                ensure_size!(in: dst, size: 1);
                dst.write_u8(MSG_SNDIN_DATA_INCOMING);
                Ok(())
            }
            Self::Data(p) => p.encode(dst),
            Self::FormatChange(p) => p.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::Version(p) => p.size(),
            Self::Formats(p) => p.size(),
            Self::Open(p) => p.size(),
            Self::OpenReply(p) => p.size(),
            Self::DataIncoming => 1,
            Self::Data(p) => p.size(),
            Self::FormatChange(p) => p.size(),
        }
    }
}

impl<'de> Decode<'de> for RdpeaiPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let message_id = src.read_u8();
        match message_id {
            MSG_SNDIN_VERSION => Ok(Self::Version(VersionPdu::decode_body(src)?)),
            MSG_SNDIN_FORMATS => Ok(Self::Formats(FormatsPdu::decode_body(src)?)),
            MSG_SNDIN_OPEN => Ok(Self::Open(OpenPdu::decode_body(src)?)),
            MSG_SNDIN_OPEN_REPLY => Ok(Self::OpenReply(OpenReplyPdu::decode_body(src)?)),
            MSG_SNDIN_DATA_INCOMING => Ok(Self::DataIncoming),
            MSG_SNDIN_DATA => Ok(Self::Data(DataPdu::decode_body(src)?)),
            MSG_SNDIN_FORMATCHANGE => Ok(Self::FormatChange(FormatChangePdu::decode_body(src)?)),
            _ => Err(invalid_field_err!("MessageId", "unknown AUDIO_INPUT message id")),
        }
    }
}

impl DvcEncode for RdpeaiPdu {}

/// Version PDU (`MSG_SNDIN_VERSION`).
///
/// [MS-RDPEAI] 2.2.2.1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionPdu {
    pub version: Version,
}

impl VersionPdu {
    const NAME: &'static str = "MSG_SNDIN_VERSION";
    const FIXED_PART_SIZE: usize = 1 /* Header */ + 4 /* Version */;

    pub fn new(version: Version) -> Self {
        Self { version }
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        let version = Version::try_from(src.read_u32())?;
        Ok(Self { version })
    }
}

impl Encode for VersionPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u8(MSG_SNDIN_VERSION);
        dst.write_u32(self.version.into());
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// Sound Formats PDU (`MSG_SNDIN_FORMATS`).
///
/// [MS-RDPEAI] 2.2.2.2
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatsPdu {
    pub formats: Vec<AudioFormat>,
    /// Server→client: arbitrary/ignored. Client→server: size of PDU excluding ExtraData.
    pub cb_size_formats_packet: u32,
    /// Optional trailing data; ignored on receive, not sent by default.
    pub extra_data: Vec<u8>,
}

impl FormatsPdu {
    const NAME: &'static str = "MSG_SNDIN_FORMATS";
    const FIXED_PART_SIZE: usize = 1 /* Header */ + 4 /* NumFormats */ + 4 /* cbSizeFormatsPacket */;

    /// Build a client→server formats PDU with the correct `cbSizeFormatsPacket`.
    pub fn client(formats: Vec<AudioFormat>) -> Self {
        let mut pdu = Self {
            formats,
            cb_size_formats_packet: 0,
            extra_data: Vec::new(),
        };
        // INVARIANT: formats PDU size fits in u32 for any realistic format table.
        pdu.cb_size_formats_packet = u32::try_from(pdu.size_without_extra()).unwrap_or(u32::MAX);
        pdu
    }

    /// Build a server→client formats PDU (`cbSizeFormatsPacket` is arbitrary).
    pub fn server(formats: Vec<AudioFormat>) -> Self {
        Self {
            formats,
            cb_size_formats_packet: 0,
            extra_data: Vec::new(),
        }
    }

    fn size_without_extra(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.formats.iter().map(AudioFormat::size).sum::<usize>())
            .expect("never overflow")
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 8);
        let num_formats = cast_length!("NumFormats", src.read_u32())?;
        let cb_size_formats_packet = src.read_u32();

        // Grow from successfully decoded formats only — do not trust NumFormats for allocation.
        let mut formats = Vec::new();
        for _ in 0..num_formats {
            formats.push(AudioFormat::decode(src)?);
        }

        // Remaining bytes are ExtraData (MAY be present; MUST be ignored).
        let extra_data = src.read_remaining().to_vec();

        Ok(Self {
            formats,
            cb_size_formats_packet,
            extra_data,
        })
    }
}

impl Encode for FormatsPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(MSG_SNDIN_FORMATS);
        dst.write_u32(cast_length!("NumFormats", self.formats.len())?);
        dst.write_u32(self.cb_size_formats_packet);
        for fmt in &self.formats {
            fmt.encode(dst)?;
        }
        if !self.extra_data.is_empty() {
            dst.write_slice(&self.extra_data);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.size_without_extra()
            .checked_add(self.extra_data.len())
            .expect("never overflow")
    }
}

/// Open PDU (`MSG_SNDIN_OPEN`).
///
/// [MS-RDPEAI] 2.2.2.3
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPdu {
    pub frames_per_packet: u32,
    pub initial_format: u32,
    pub capture_format: AudioFormat,
}

impl OpenPdu {
    const NAME: &'static str = "MSG_SNDIN_OPEN";
    const FIXED_PART_SIZE: usize = 1 /* Header */ + 4 /* FramesPerPacket */ + 4 /* initialFormat */;

    /// PCM packet size for each Data PDU, if within operational bounds.
    ///
    /// Prefer `nBlockAlign * FramesPerPacket`; fall back to the MS-RDPEAI 16-bit formula
    /// `nChannels * 2 * FramesPerPacket` when block align is zero.
    pub fn data_packet_size(&self) -> Option<usize> {
        if self.frames_per_packet == 0 || self.frames_per_packet > MAX_FRAMES_PER_PACKET {
            return None;
        }

        let channels = usize::from(self.capture_format.n_channels);
        let bits = usize::from(self.capture_format.bits_per_sample);
        let frames = usize::try_from(self.frames_per_packet).ok()?;
        let block = usize::from(self.capture_format.n_block_align);

        let size = if block > 0 {
            frames.checked_mul(block)?
        } else {
            let bytes_per_sample = bits.checked_div(8)?.max(1);
            frames.checked_mul(channels)?.checked_mul(bytes_per_sample)?
        };

        (size > 0 && size <= MAX_DATA_PACKET_SIZE).then_some(size)
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 8);
        let frames_per_packet = src.read_u32();
        let initial_format = src.read_u32();
        let capture_format = AudioFormat::decode(src)?;
        Ok(Self {
            frames_per_packet,
            initial_format,
            capture_format,
        })
    }
}

impl Encode for OpenPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(MSG_SNDIN_OPEN);
        dst.write_u32(self.frames_per_packet);
        dst.write_u32(self.initial_format);
        self.capture_format.encode(dst)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(self.capture_format.size())
            .expect("never overflow")
    }
}

/// Open Reply PDU (`MSG_SNDIN_OPEN_REPLY`).
///
/// [MS-RDPEAI] 2.2.2.4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenReplyPdu {
    /// HRESULT status of opening the capture device.
    pub result: i32,
}

impl OpenReplyPdu {
    const NAME: &'static str = "MSG_SNDIN_OPEN_REPLY";
    const FIXED_PART_SIZE: usize = 1 /* Header */ + 4 /* Result */;

    pub const S_OK: i32 = 0;
    /// `E_FAIL` (0x80004005) as a signed HRESULT.
    pub const E_FAIL: i32 = -2147467259;

    pub fn ok() -> Self {
        Self { result: Self::S_OK }
    }

    pub fn fail() -> Self {
        Self { result: Self::E_FAIL }
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        Ok(Self { result: src.read_i32() })
    }
}

impl Encode for OpenReplyPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u8(MSG_SNDIN_OPEN_REPLY);
        dst.write_i32(self.result);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// Data PDU (`MSG_SNDIN_DATA`).
///
/// [MS-RDPEAI] 2.2.3.2
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPdu {
    pub data: Vec<u8>,
}

impl DataPdu {
    const NAME: &'static str = "MSG_SNDIN_DATA";

    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(Self {
            data: src.read_remaining().to_vec(),
        })
    }
}

impl Encode for DataPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(MSG_SNDIN_DATA);
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        1usize.checked_add(self.data.len()).expect("never overflow")
    }
}

/// Format Change PDU (`MSG_SNDIN_FORMATCHANGE`).
///
/// [MS-RDPEAI] 2.2.4.1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatChangePdu {
    pub new_format: u32,
}

impl FormatChangePdu {
    const NAME: &'static str = "MSG_SNDIN_FORMATCHANGE";
    const FIXED_PART_SIZE: usize = 1 /* Header */ + 4 /* NewFormat */;

    pub fn new(new_format: u32) -> Self {
        Self { new_format }
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        Ok(Self {
            new_format: src.read_u32(),
        })
    }
}

impl Encode for FormatChangePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u8(MSG_SNDIN_FORMATCHANGE);
        dst.write_u32(self.new_format);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// Helper: standard PCM WAVEFORMATEX values.
pub fn pcm_format(channels: u16, samples_per_sec: u32, bits_per_sample: u16) -> AudioFormat {
    let bytes_per_sample = bits_per_sample / 8;
    let n_block_align = channels.saturating_mul(bytes_per_sample);
    AudioFormat {
        format: WaveFormat::PCM,
        n_channels: channels,
        n_samples_per_sec: samples_per_sec,
        n_avg_bytes_per_sec: samples_per_sec.saturating_mul(u32::from(n_block_align)),
        n_block_align,
        bits_per_sample,
        data: None,
    }
}
