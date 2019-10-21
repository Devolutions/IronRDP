use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{
    client_info, ClientConfirmActive, ControlPdu, MonitorLayoutPdu, RdpError, ServerDemandActive,
    SynchronizePdu,
};
use crate::rdp::finalization_messages::FontPdu;
use crate::PduParsing;

const BASIC_SECURITY_HEADER_SIZE: usize = 4;
const SHARE_CONTROL_HEADER_MASK: u16 = 0xf;
const SHARE_DATA_HEADER_MASK: u8 = 0xf;
const SHARE_CONTROL_HEADER_SIZE: usize = 2 * 3 + 4;

const PROTOCOL_VERSION: u16 = 0x10;

// ShareDataHeader
const PADDING_FIELD_SIZE: usize = 1;
const STREAM_ID_FIELD_SIZE: usize = 1;
const UNCOMPRESSED_LENGTH_FIELD_SIZE: usize = 2;
const PDU_TYPE_FIELD_SIZE: usize = 1;
const COMPRESSION_TYPE_FIELD_SIZE: usize = 1;
const COMPRESSED_LENGTH_FIELD_SIZE: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct BasicSecurityHeader {
    pub flags: BasicSecurityHeaderFlags,
}

impl BasicSecurityHeader {
    pub fn new(flags: BasicSecurityHeaderFlags) -> Self {
        Self { flags }
    }
}

impl PduParsing for BasicSecurityHeader {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let flags = BasicSecurityHeaderFlags::from_bits(stream.read_u16::<LittleEndian>()?)
            .ok_or(RdpError::InvalidSecurityHeader)?;
        let _flags_hi = stream.read_u16::<LittleEndian>()?; // unused

        Ok(Self { flags })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.flags.bits())?;
        stream.write_u16::<LittleEndian>(0)?; // flags_hi

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BASIC_SECURITY_HEADER_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShareControlHeader {
    pub share_control_pdu: ShareControlPdu,
    pub pdu_source: u16,
    pub share_id: u32,
}

impl ShareControlHeader {
    pub fn new(share_control_pdu: ShareControlPdu, pdu_source: u16, share_id: u32) -> Self {
        Self {
            share_control_pdu,
            pdu_source,
            share_id,
        }
    }
}

impl PduParsing for ShareControlHeader {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _total_length = stream.read_u16::<LittleEndian>()?;
        let pdu_type_with_version = stream.read_u16::<LittleEndian>()?;
        let pdu_source = stream.read_u16::<LittleEndian>()?;
        let share_id = stream.read_u32::<LittleEndian>()?;

        let pdu_type =
            ShareControlPduType::from_u16(pdu_type_with_version & SHARE_CONTROL_HEADER_MASK)
                .ok_or_else(|| {
                    RdpError::InvalidShareControlHeader(String::from("Invalid pdu type"))
                })?;
        let pdu_version = pdu_type_with_version & !SHARE_CONTROL_HEADER_MASK;
        if pdu_version != PROTOCOL_VERSION {
            return Err(RdpError::InvalidShareControlHeader(format!(
                "Invalid PDU version: {}",
                pdu_version
            )));
        }

        let share_pdu = ShareControlPdu::from_type(&mut stream, pdu_type)?;

        Ok(Self {
            share_control_pdu: share_pdu,
            pdu_source,
            share_id,
        })
    }
    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let pdu_type_with_version =
            PROTOCOL_VERSION | self.share_control_pdu.share_header_type().to_u16().unwrap();

        stream.write_u16::<LittleEndian>(
            (self.share_control_pdu.buffer_length() + SHARE_CONTROL_HEADER_SIZE) as u16,
        )?;
        stream.write_u16::<LittleEndian>(pdu_type_with_version)?;
        stream.write_u16::<LittleEndian>(self.pdu_source)?;
        stream.write_u32::<LittleEndian>(self.share_id)?;

        self.share_control_pdu.to_buffer(&mut stream)
    }
    fn buffer_length(&self) -> usize {
        SHARE_CONTROL_HEADER_SIZE + self.share_control_pdu.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq)]
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
    pub fn from_type(
        mut stream: impl io::Read,
        share_type: ShareControlPduType,
    ) -> Result<Self, RdpError> {
        match share_type {
            ShareControlPduType::DemandActivePdu => Ok(ShareControlPdu::ServerDemandActive(
                ServerDemandActive::from_buffer(&mut stream)?,
            )),
            ShareControlPduType::ConfirmActivePdu => Ok(ShareControlPdu::ClientConfirmActive(
                ClientConfirmActive::from_buffer(&mut stream)?,
            )),
            ShareControlPduType::DataPdu => Ok(ShareControlPdu::Data(
                ShareDataHeader::from_buffer(&mut stream)?,
            )),
            _ => Err(RdpError::UnexpectedShareControlPdu(share_type)),
        }
    }
    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), RdpError> {
        match self {
            ShareControlPdu::ServerDemandActive(pdu) => {
                pdu.to_buffer(&mut stream).map_err(RdpError::from)
            }
            ShareControlPdu::ClientConfirmActive(pdu) => {
                pdu.to_buffer(&mut stream).map_err(RdpError::from)
            }
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

#[derive(Debug, Clone, PartialEq)]
pub struct ShareDataHeader {
    pub share_data_pdu: ShareDataPdu,
    pub stream_priority: StreamPriority,
    pub compression_flags: CompressionFlags,
    pub compression_type: client_info::CompressionType,
}

impl ShareDataHeader {
    pub fn new(
        share_data_pdu: ShareDataPdu,
        stream_priority: StreamPriority,
        compression_flags: CompressionFlags,
        compression_type: client_info::CompressionType,
    ) -> Self {
        Self {
            share_data_pdu,
            stream_priority,
            compression_flags,
            compression_type,
        }
    }
}

impl PduParsing for ShareDataHeader {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _padding = stream.read_u8()?;
        let stream_priority = StreamPriority::from_u8(stream.read_u8()?).ok_or_else(|| {
            RdpError::InvalidShareDataHeader(String::from("Invalid stream priority"))
        })?;
        let _uncompressed_length = stream.read_u16::<LittleEndian>()?;
        let pdu_type = ShareDataPduType::from_u8(stream.read_u8()?)
            .ok_or_else(|| RdpError::InvalidShareDataHeader(String::from("Invalid pdu type")))?;
        let compression_flags_with_type = stream.read_u8()?;

        let compression_flags = CompressionFlags::from_bits_truncate(
            compression_flags_with_type & SHARE_DATA_HEADER_MASK,
        );
        let compression_type = client_info::CompressionType::from_u8(
            compression_flags_with_type & !SHARE_DATA_HEADER_MASK,
        )
        .ok_or_else(|| {
            RdpError::InvalidShareDataHeader(String::from("Invalid compression type"))
        })?;
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
            let compression_flags_with_type =
                self.compression_flags.bits() | self.compression_type.to_u8().unwrap();

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

#[derive(Debug, Clone, PartialEq)]
pub enum ShareDataPdu {
    Synchronize(SynchronizePdu),
    Control(ControlPdu),
    FontList(FontPdu),
    FontMap(FontPdu),
    MonitorLayout(MonitorLayoutPdu),
}

impl ShareDataPdu {
    pub fn as_short_name(&self) -> &str {
        match self {
            ShareDataPdu::Synchronize(_) => "Synchronize PDU",
            ShareDataPdu::Control(_) => "Control PDU",
            ShareDataPdu::FontList(_) => "FontList PDU",
            ShareDataPdu::FontMap(_) => "Font Map PDU",
            ShareDataPdu::MonitorLayout(_) => "Monitor Layout PDU",
        }
    }
}

impl ShareDataPdu {
    pub fn from_type(
        mut stream: impl io::Read,
        share_type: ShareDataPduType,
    ) -> Result<Self, RdpError> {
        match share_type {
            ShareDataPduType::Synchronize => Ok(ShareDataPdu::Synchronize(
                SynchronizePdu::from_buffer(&mut stream)?,
            )),
            ShareDataPduType::Control => {
                Ok(ShareDataPdu::Control(ControlPdu::from_buffer(&mut stream)?))
            }
            ShareDataPduType::FontList => {
                Ok(ShareDataPdu::FontList(FontPdu::from_buffer(&mut stream)?))
            }
            ShareDataPduType::FontMap => {
                Ok(ShareDataPdu::FontMap(FontPdu::from_buffer(&mut stream)?))
            }
            ShareDataPduType::MonitorLayoutPdu => Ok(ShareDataPdu::MonitorLayout(
                MonitorLayoutPdu::from_buffer(&mut stream)?,
            )),
            ShareDataPduType::Update
            | ShareDataPduType::Pointer
            | ShareDataPduType::Input
            | ShareDataPduType::RefreshRectangle
            | ShareDataPduType::PlaySound
            | ShareDataPduType::SuppressOutput
            | ShareDataPduType::ShutdownRequest
            | ShareDataPduType::ShutdownDenied
            | ShareDataPduType::SaveSessionInfo
            | ShareDataPduType::SetKeyboardIndicators
            | ShareDataPduType::BitmapCachePersistentList
            | ShareDataPduType::BitmapCacheErrorPdu
            | ShareDataPduType::SetKeyboardImeStatus
            | ShareDataPduType::OffscreenCacheErrorPdu
            | ShareDataPduType::SetErrorInfoPdu
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
        }
    }
    pub fn buffer_length(&self) -> usize {
        match self {
            ShareDataPdu::Synchronize(pdu) => pdu.buffer_length(),
            ShareDataPdu::Control(pdu) => pdu.buffer_length(),
            ShareDataPdu::FontList(pdu) | ShareDataPdu::FontMap(pdu) => pdu.buffer_length(),
            ShareDataPdu::MonitorLayout(pdu) => pdu.buffer_length(),
        }
    }
    pub fn share_header_type(&self) -> ShareDataPduType {
        match self {
            ShareDataPdu::Synchronize(_) => ShareDataPduType::Synchronize,
            ShareDataPdu::Control(_) => ShareDataPduType::Control,
            ShareDataPdu::FontList(_) => ShareDataPduType::FontList,
            ShareDataPdu::FontMap(_) => ShareDataPduType::FontMap,
            ShareDataPdu::MonitorLayout(_) => ShareDataPduType::MonitorLayoutPdu,
        }
    }
}

bitflags! {
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

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum StreamPriority {
    Undefined = 0,
    Low = 1,
    Medium = 2,
    High = 4,
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum ShareControlPduType {
    DemandActivePdu = 0x1,
    ConfirmActivePdu = 0x3,
    DeactivateAllPdu = 0x6,
    DataPdu = 0x7,
    ServerRedirect = 0xa,
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
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
}

bitflags! {
    pub struct CompressionFlags: u8 {
        const COMPRESSED = 0x20;
        const AT_FRONT = 0x40;
        const FLUSHED = 0x80;
    }
}
