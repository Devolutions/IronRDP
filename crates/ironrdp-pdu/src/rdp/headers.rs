use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::codecs::rfx::FrameAcknowledgePdu;
use crate::cursor::{ReadCursor, WriteCursor};
use crate::input::InputEventPdu;
use crate::rdp::capability_sets::{ClientConfirmActive, ServerDemandActive};
use crate::rdp::finalization_messages::{ControlPdu, FontPdu, MonitorLayoutPdu, SynchronizePdu};
use crate::rdp::refresh_rectangle::RefreshRectanglePdu;
use crate::rdp::server_error_info::ServerSetErrorInfoPdu;
use crate::rdp::session_info::SaveSessionInfoPdu;
use crate::rdp::suppress_output::SuppressOutputPdu;
use crate::rdp::{client_info, RdpError};
use crate::{PduDecode, PduEncode, PduParsing, PduResult};

pub const BASIC_SECURITY_HEADER_SIZE: usize = 4;
pub const SHARE_DATA_HEADER_COMPRESSION_MASK: u8 = 0xF;
const SHARE_CONTROL_HEADER_MASK: u16 = 0xF;
const SHARE_CONTROL_HEADER_SIZE: usize = 2 * 3 + 4;

const PROTOCOL_VERSION: u16 = 0x10;

// ShareDataHeader
const PADDING_FIELD_SIZE: usize = 1;
const STREAM_ID_FIELD_SIZE: usize = 1;
const UNCOMPRESSED_LENGTH_FIELD_SIZE: usize = 2;
const PDU_TYPE_FIELD_SIZE: usize = 1;
const COMPRESSION_TYPE_FIELD_SIZE: usize = 1;
const COMPRESSED_LENGTH_FIELD_SIZE: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicSecurityHeader {
    pub flags: BasicSecurityHeaderFlags,
}

impl BasicSecurityHeader {
    const NAME: &'static str = "BasicSecurityHeader";

    pub const FIXED_PART_SIZE: usize = BASIC_SECURITY_HEADER_SIZE;
}

impl PduEncode for BasicSecurityHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.flags.bits());
        dst.write_u16(0); // flags_hi
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for BasicSecurityHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = BasicSecurityHeaderFlags::from_bits(src.read_u16())
            .ok_or(invalid_message_err!("securityHeader", "invalid basic security header"))?;
        let _flags_hi = src.read_u16(); // unused

        Ok(Self { flags })
    }
}

impl_pdu_parsing!(BasicSecurityHeader);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareControlHeader {
    pub share_control_pdu: ShareControlPdu,
    pub pdu_source: u16,
    pub share_id: u32,
}

impl PduParsing for ShareControlHeader {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let total_length = stream.read_u16::<LittleEndian>()? as usize;
        let pdu_type_with_version = stream.read_u16::<LittleEndian>()?;
        let pdu_source = stream.read_u16::<LittleEndian>()?;
        let share_id = stream.read_u32::<LittleEndian>()?;

        let pdu_type = ShareControlPduType::from_u16(pdu_type_with_version & SHARE_CONTROL_HEADER_MASK)
            .ok_or_else(|| RdpError::InvalidShareControlHeader(format!("invalid pdu type: {pdu_type_with_version}")))?;
        let pdu_version = pdu_type_with_version & !SHARE_CONTROL_HEADER_MASK;
        if pdu_version != PROTOCOL_VERSION {
            return Err(RdpError::InvalidShareControlHeader(format!(
                "Invalid PDU version: {pdu_version}"
            )));
        }

        let share_pdu = ShareControlPdu::from_type(&mut stream, pdu_type)?;
        let header = Self {
            share_control_pdu: share_pdu,
            pdu_source,
            share_id,
        };

        if pdu_type == ShareControlPduType::DataPdu {
            // Some windows version have an issue where
            // there is some padding not part of the inner unit.
            // Consume that data
            let header_length = header.buffer_length();

            if header_length != total_length {
                if total_length < header_length {
                    return Err(RdpError::NotEnoughBytes);
                }

                let padding = total_length - header_length;
                let mut data = vec![0u8; padding];
                stream.read_exact(data.as_mut())?;
            }
        }

        Ok(header)
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let pdu_type_with_version = PROTOCOL_VERSION | self.share_control_pdu.share_header_type().to_u16().unwrap();

        stream
            .write_u16::<LittleEndian>((self.share_control_pdu.buffer_length() + SHARE_CONTROL_HEADER_SIZE) as u16)?;
        stream.write_u16::<LittleEndian>(pdu_type_with_version)?;
        stream.write_u16::<LittleEndian>(self.pdu_source)?;
        stream.write_u32::<LittleEndian>(self.share_id)?;

        self.share_control_pdu.to_buffer(&mut stream)
    }

    fn buffer_length(&self) -> usize {
        SHARE_CONTROL_HEADER_SIZE + self.share_control_pdu.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareControlPdu {
    ServerDemandActive(ServerDemandActive),
    ClientConfirmActive(ClientConfirmActive),
    Data(ShareDataHeader),
}

impl ShareControlPdu {
    pub fn as_short_name(&self) -> &str {
        match self {
            ShareControlPdu::ServerDemandActive(_) => "Server Demand Active PDU",
            ShareControlPdu::ClientConfirmActive(_) => "Client Confirm Active PDU",
            ShareControlPdu::Data(_) => "Data PDU",
        }
    }
}

impl ShareControlPdu {
    pub fn from_type(mut stream: impl io::Read, share_type: ShareControlPduType) -> Result<Self, RdpError> {
        match share_type {
            ShareControlPduType::DemandActivePdu => Ok(ShareControlPdu::ServerDemandActive(
                ServerDemandActive::from_buffer(&mut stream)?,
            )),
            ShareControlPduType::ConfirmActivePdu => Ok(ShareControlPdu::ClientConfirmActive(
                ClientConfirmActive::from_buffer(&mut stream)?,
            )),
            ShareControlPduType::DataPdu => Ok(ShareControlPdu::Data(ShareDataHeader::from_buffer(&mut stream)?)),
            _ => Err(RdpError::UnexpectedShareControlPdu(share_type)),
        }
    }
    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), RdpError> {
        match self {
            ShareControlPdu::ServerDemandActive(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareControlPdu::ClientConfirmActive(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareControlPdu::Data(share_data_header) => share_data_header.to_buffer(&mut stream),
        }
    }
    pub fn buffer_length(&self) -> usize {
        match self {
            ShareControlPdu::ServerDemandActive(pdu) => pdu.buffer_length(),
            ShareControlPdu::ClientConfirmActive(pdu) => pdu.buffer_length(),
            ShareControlPdu::Data(share_data_header) => share_data_header.buffer_length(),
        }
    }
    pub fn share_header_type(&self) -> ShareControlPduType {
        match self {
            ShareControlPdu::ServerDemandActive(_) => ShareControlPduType::DemandActivePdu,
            ShareControlPdu::ClientConfirmActive(_) => ShareControlPduType::ConfirmActivePdu,
            ShareControlPdu::Data(_) => ShareControlPduType::DataPdu,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareDataHeader {
    pub share_data_pdu: ShareDataPdu,
    pub stream_priority: StreamPriority,
    pub compression_flags: CompressionFlags,
    pub compression_type: client_info::CompressionType,
}

impl PduParsing for ShareDataHeader {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _padding = stream.read_u8()?;
        let stream_priority = StreamPriority::from_u8(stream.read_u8()?)
            .ok_or_else(|| RdpError::InvalidShareDataHeader(String::from("Invalid stream priority")))?;
        let _uncompressed_length = stream.read_u16::<LittleEndian>()?;
        let pdu_type = ShareDataPduType::from_u8(stream.read_u8()?)
            .ok_or_else(|| RdpError::InvalidShareDataHeader(String::from("Invalid pdu type")))?;
        let compression_flags_with_type = stream.read_u8()?;

        let compression_flags =
            CompressionFlags::from_bits_truncate(compression_flags_with_type & !SHARE_DATA_HEADER_COMPRESSION_MASK);
        let compression_type =
            client_info::CompressionType::from_u8(compression_flags_with_type & SHARE_DATA_HEADER_COMPRESSION_MASK)
                .ok_or_else(|| RdpError::InvalidShareDataHeader(String::from("Invalid compression type")))?;
        let _compressed_length = stream.read_u16::<LittleEndian>()?;

        let share_data_pdu = ShareDataPdu::from_type(&mut stream, pdu_type)?;

        Ok(Self {
            share_data_pdu,
            stream_priority,
            compression_flags,
            compression_type,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        if self.compression_flags.is_empty() {
            let compression_flags_with_type = self.compression_flags.bits() | self.compression_type.to_u8().unwrap();

            stream.write_u8(0)?; // padding
            stream.write_u8(self.stream_priority.to_u8().unwrap())?;
            stream.write_u16::<LittleEndian>(
                (self.share_data_pdu.buffer_length()
                    + PDU_TYPE_FIELD_SIZE
                    + COMPRESSION_TYPE_FIELD_SIZE
                    + COMPRESSED_LENGTH_FIELD_SIZE) as u16,
            )?;
            stream.write_u8(self.share_data_pdu.share_header_type().to_u8().unwrap())?;
            stream.write_u8(compression_flags_with_type)?;
            stream.write_u16::<LittleEndian>(0)?; // compressed length

            self.share_data_pdu.to_buffer(&mut stream)
        } else {
            Err(RdpError::InvalidShareDataHeader(String::from(
                "Compression is not implemented",
            )))
        }
    }

    fn buffer_length(&self) -> usize {
        PADDING_FIELD_SIZE
            + STREAM_ID_FIELD_SIZE
            + UNCOMPRESSED_LENGTH_FIELD_SIZE
            + PDU_TYPE_FIELD_SIZE
            + COMPRESSION_TYPE_FIELD_SIZE
            + COMPRESSED_LENGTH_FIELD_SIZE
            + self.share_data_pdu.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareDataPdu {
    Synchronize(SynchronizePdu),
    Control(ControlPdu),
    FontList(FontPdu),
    FontMap(FontPdu),
    MonitorLayout(MonitorLayoutPdu),
    SaveSessionInfo(SaveSessionInfoPdu),
    FrameAcknowledge(FrameAcknowledgePdu),
    ServerSetErrorInfo(ServerSetErrorInfoPdu),
    Input(InputEventPdu),
    ShutdownRequest,
    ShutdownDenied,
    SuppressOutput(SuppressOutputPdu),
    RefreshRectangle(RefreshRectanglePdu),
}

impl ShareDataPdu {
    pub fn as_short_name(&self) -> &str {
        match self {
            ShareDataPdu::Synchronize(_) => "Synchronize PDU",
            ShareDataPdu::Control(_) => "Control PDU",
            ShareDataPdu::FontList(_) => "FontList PDU",
            ShareDataPdu::FontMap(_) => "Font Map PDU",
            ShareDataPdu::MonitorLayout(_) => "Monitor Layout PDU",
            ShareDataPdu::SaveSessionInfo(_) => "Save session info PDU",
            ShareDataPdu::FrameAcknowledge(_) => "Frame Acknowledge PDU",
            ShareDataPdu::ServerSetErrorInfo(_) => "Server Set Error Info PDU",
            ShareDataPdu::Input(_) => "Server Input PDU",
            ShareDataPdu::ShutdownRequest => "Shutdown Request PDU",
            ShareDataPdu::ShutdownDenied => "Shutdown Denied PDU",
            ShareDataPdu::SuppressOutput(_) => "Suppress Output PDU",
            ShareDataPdu::RefreshRectangle(_) => "Refresh Rectangle PDU",
        }
    }
}

impl ShareDataPdu {
    pub fn from_type(mut stream: impl io::Read, share_type: ShareDataPduType) -> Result<Self, RdpError> {
        match share_type {
            ShareDataPduType::Synchronize => Ok(ShareDataPdu::Synchronize(SynchronizePdu::from_buffer(&mut stream)?)),
            ShareDataPduType::Control => Ok(ShareDataPdu::Control(ControlPdu::from_buffer(&mut stream)?)),
            ShareDataPduType::FontList => Ok(ShareDataPdu::FontList(FontPdu::from_buffer(&mut stream)?)),
            ShareDataPduType::FontMap => Ok(ShareDataPdu::FontMap(FontPdu::from_buffer(&mut stream)?)),
            ShareDataPduType::MonitorLayoutPdu => {
                Ok(ShareDataPdu::MonitorLayout(MonitorLayoutPdu::from_buffer(&mut stream)?))
            }
            ShareDataPduType::SaveSessionInfo => Ok(ShareDataPdu::SaveSessionInfo(SaveSessionInfoPdu::from_buffer(
                &mut stream,
            )?)),
            ShareDataPduType::FrameAcknowledgePdu => Ok(ShareDataPdu::FrameAcknowledge(
                FrameAcknowledgePdu::from_buffer(&mut stream)?,
            )),
            ShareDataPduType::SetErrorInfoPdu => Ok(ShareDataPdu::ServerSetErrorInfo(
                ServerSetErrorInfoPdu::from_buffer(&mut stream)?,
            )),
            ShareDataPduType::Input => Ok(ShareDataPdu::Input(InputEventPdu::from_buffer(&mut stream)?)),
            ShareDataPduType::ShutdownRequest => Ok(ShareDataPdu::ShutdownRequest),
            ShareDataPduType::ShutdownDenied => Ok(ShareDataPdu::ShutdownDenied),
            ShareDataPduType::SuppressOutput => Ok(ShareDataPdu::SuppressOutput(SuppressOutputPdu::from_buffer(
                &mut stream,
            )?)),
            ShareDataPduType::RefreshRectangle => Ok(ShareDataPdu::RefreshRectangle(RefreshRectanglePdu::from_buffer(
                &mut stream,
            )?)),
            ShareDataPduType::Update
            | ShareDataPduType::Pointer
            | ShareDataPduType::PlaySound
            | ShareDataPduType::SetKeyboardIndicators
            | ShareDataPduType::BitmapCachePersistentList
            | ShareDataPduType::BitmapCacheErrorPdu
            | ShareDataPduType::SetKeyboardImeStatus
            | ShareDataPduType::OffscreenCacheErrorPdu
            | ShareDataPduType::DrawNineGridErrorPdu
            | ShareDataPduType::DrawGdiPusErrorPdu
            | ShareDataPduType::ArcStatusPdu
            | ShareDataPduType::StatusInfoPdu => Err(RdpError::UnexpectedShareDataPdu(share_type)),
        }
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), RdpError> {
        match self {
            ShareDataPdu::Synchronize(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::Control(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::FontList(pdu) | ShareDataPdu::FontMap(pdu) => {
                pdu.to_buffer(&mut stream).map_err(RdpError::from)
            }
            ShareDataPdu::MonitorLayout(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::SaveSessionInfo(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::FrameAcknowledge(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::ServerSetErrorInfo(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::Input(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::ShutdownRequest | ShareDataPdu::ShutdownDenied => Ok(()),
            ShareDataPdu::SuppressOutput(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
            ShareDataPdu::RefreshRectangle(pdu) => pdu.to_buffer(&mut stream).map_err(RdpError::from),
        }
    }

    pub fn buffer_length(&self) -> usize {
        match self {
            ShareDataPdu::Synchronize(pdu) => pdu.buffer_length(),
            ShareDataPdu::Control(pdu) => pdu.buffer_length(),
            ShareDataPdu::FontList(pdu) | ShareDataPdu::FontMap(pdu) => pdu.buffer_length(),
            ShareDataPdu::MonitorLayout(pdu) => pdu.buffer_length(),
            ShareDataPdu::SaveSessionInfo(pdu) => pdu.buffer_length(),
            ShareDataPdu::FrameAcknowledge(pdu) => pdu.buffer_length(),
            ShareDataPdu::ServerSetErrorInfo(pdu) => pdu.buffer_length(),
            ShareDataPdu::Input(pdu) => pdu.buffer_length(),
            ShareDataPdu::ShutdownRequest | ShareDataPdu::ShutdownDenied => 0,
            ShareDataPdu::SuppressOutput(pdu) => pdu.buffer_length(),
            ShareDataPdu::RefreshRectangle(pdu) => pdu.buffer_length(),
        }
    }
    pub fn share_header_type(&self) -> ShareDataPduType {
        match self {
            ShareDataPdu::Synchronize(_) => ShareDataPduType::Synchronize,
            ShareDataPdu::Control(_) => ShareDataPduType::Control,
            ShareDataPdu::FontList(_) => ShareDataPduType::FontList,
            ShareDataPdu::FontMap(_) => ShareDataPduType::FontMap,
            ShareDataPdu::MonitorLayout(_) => ShareDataPduType::MonitorLayoutPdu,
            ShareDataPdu::SaveSessionInfo(_) => ShareDataPduType::SaveSessionInfo,
            ShareDataPdu::FrameAcknowledge(_) => ShareDataPduType::FrameAcknowledgePdu,
            ShareDataPdu::ServerSetErrorInfo(_) => ShareDataPduType::SetErrorInfoPdu,
            ShareDataPdu::Input(_) => ShareDataPduType::Input,
            ShareDataPdu::ShutdownRequest => ShareDataPduType::ShutdownRequest,
            ShareDataPdu::ShutdownDenied => ShareDataPduType::ShutdownDenied,
            ShareDataPdu::SuppressOutput(_) => ShareDataPduType::SuppressOutput,
            ShareDataPdu::RefreshRectangle(_) => ShareDataPduType::RefreshRectangle,
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct BasicSecurityHeaderFlags: u16 {
        const EXCHANGE_PKT = 0x0001;
        const TRANSPORT_REQ = 0x0002;
        const TRANSPORT_RSP = 0x0004;
        const ENCRYPT = 0x0008;
        const RESET_SEQNO = 0x0010;
        const IGNORE_SEQNO = 0x0020;
        const INFO_PKT = 0x0040;
        const LICENSE_PKT = 0x0080;
        const LICENSE_ENCRYPT_CS = 0x0100;
        const LICENSE_ENCRYPT_SC = 0x0200;
        const REDIRECTION_PKT = 0x0400;
        const SECURE_CHECKSUM = 0x0800;
        const AUTODETECT_REQ = 0x1000;
        const AUTODETECT_RSP = 0x2000;
        const HEARTBEAT = 0x4000;
        const FLAGSHI_VALID = 0x8000;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum StreamPriority {
    Undefined = 0,
    Low = 1,
    Medium = 2,
    High = 4,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ShareControlPduType {
    DemandActivePdu = 0x1,
    ConfirmActivePdu = 0x3,
    DeactivateAllPdu = 0x6,
    DataPdu = 0x7,
    ServerRedirect = 0xa,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
#[repr(u8)]
pub enum ShareDataPduType {
    Update = 0x02,
    Control = 0x14,
    Pointer = 0x1b,
    Input = 0x1c,
    Synchronize = 0x1f,
    RefreshRectangle = 0x21,
    PlaySound = 0x22,
    SuppressOutput = 0x23,
    ShutdownRequest = 0x24,
    ShutdownDenied = 0x25,
    SaveSessionInfo = 0x26,
    FontList = 0x27,
    FontMap = 0x28,
    SetKeyboardIndicators = 0x29,
    BitmapCachePersistentList = 0x2b,
    BitmapCacheErrorPdu = 0x2c,
    SetKeyboardImeStatus = 0x2d,
    OffscreenCacheErrorPdu = 0x2e,
    SetErrorInfoPdu = 0x2f,
    DrawNineGridErrorPdu = 0x30,
    DrawGdiPusErrorPdu = 0x31,
    ArcStatusPdu = 0x32,
    StatusInfoPdu = 0x36,
    MonitorLayoutPdu = 0x37,
    FrameAcknowledgePdu = 0x38,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CompressionFlags: u8 {
        const COMPRESSED = 0x20;
        const AT_FRONT = 0x40;
        const FLUSHED = 0x80;
    }
}
