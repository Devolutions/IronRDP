use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::codecs::rfx::FrameAcknowledgePdu;
use crate::cursor::{ReadCursor, WriteCursor};
use crate::input::InputEventPdu;
use crate::rdp::capability_sets::{ClientConfirmActive, ServerDemandActive};
use crate::rdp::client_info;
use crate::rdp::finalization_messages::{ControlPdu, FontPdu, MonitorLayoutPdu, SynchronizePdu};
use crate::rdp::refresh_rectangle::RefreshRectanglePdu;
use crate::rdp::server_error_info::ServerSetErrorInfoPdu;
use crate::rdp::session_info::SaveSessionInfoPdu;
use crate::rdp::suppress_output::SuppressOutputPdu;
use crate::{PduDecode, PduEncode, PduResult};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareControlHeader {
    pub share_control_pdu: ShareControlPdu,
    pub pdu_source: u16,
    pub share_id: u32,
}

impl ShareControlHeader {
    const NAME: &'static str = "ShareControlHeader";

    const FIXED_PART_SIZE: usize = SHARE_CONTROL_HEADER_SIZE;
}

impl PduEncode for ShareControlHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let pdu_type_with_version = PROTOCOL_VERSION | self.share_control_pdu.share_header_type().to_u16().unwrap();

        dst.write_u16(cast_length!(
            "len",
            self.share_control_pdu.size() + SHARE_CONTROL_HEADER_SIZE
        )?);
        dst.write_u16(pdu_type_with_version);
        dst.write_u16(self.pdu_source);
        dst.write_u32(self.share_id);

        self.share_control_pdu.encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.share_control_pdu.size()
    }
}

impl<'de> PduDecode<'de> for ShareControlHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let total_length = src.read_u16() as usize;
        let pdu_type_with_version = src.read_u16();
        let pdu_source = src.read_u16();
        let share_id = src.read_u32();

        let pdu_type = ShareControlPduType::from_u16(pdu_type_with_version & SHARE_CONTROL_HEADER_MASK)
            .ok_or_else(|| invalid_message_err!("pdu_type", "invalid pdu type"))?;
        let pdu_version = pdu_type_with_version & !SHARE_CONTROL_HEADER_MASK;
        if pdu_version != PROTOCOL_VERSION {
            return Err(invalid_message_err!("pdu_version", "invalid PDU version"));
        }

        let share_pdu = ShareControlPdu::from_type(src, pdu_type)?;
        let header = Self {
            share_control_pdu: share_pdu,
            pdu_source,
            share_id,
        };

        if pdu_type == ShareControlPduType::DataPdu {
            // Some windows version have an issue where
            // there is some padding not part of the inner unit.
            // Consume that data
            let header_length = header.size();

            if header_length != total_length {
                if total_length < header_length {
                    return Err(not_enough_bytes_err!(total_length, header_length));
                }

                let padding = total_length - header_length;
                ensure_size!(in: src, size: padding);
                read_padding!(src, padding);
            }
        }

        Ok(header)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareControlPdu {
    ServerDemandActive(ServerDemandActive),
    ClientConfirmActive(ClientConfirmActive),
    Data(ShareDataHeader),
    ServerDeactivateAll(ServerDeactivateAll),
}

impl ShareControlPdu {
    const NAME: &'static str = "ShareControlPdu";

    pub fn as_short_name(&self) -> &str {
        match self {
            ShareControlPdu::ServerDemandActive(_) => "Server Demand Active PDU",
            ShareControlPdu::ClientConfirmActive(_) => "Client Confirm Active PDU",
            ShareControlPdu::Data(_) => "Data PDU",
            ShareControlPdu::ServerDeactivateAll(_) => "Server Deactivate All PDU",
        }
    }

    pub fn share_header_type(&self) -> ShareControlPduType {
        match self {
            ShareControlPdu::ServerDemandActive(_) => ShareControlPduType::DemandActivePdu,
            ShareControlPdu::ClientConfirmActive(_) => ShareControlPduType::ConfirmActivePdu,
            ShareControlPdu::Data(_) => ShareControlPduType::DataPdu,
            ShareControlPdu::ServerDeactivateAll(_) => ShareControlPduType::DeactivateAllPdu,
        }
    }

    pub fn from_type(src: &mut ReadCursor<'_>, share_type: ShareControlPduType) -> PduResult<Self> {
        match share_type {
            ShareControlPduType::DemandActivePdu => {
                Ok(ShareControlPdu::ServerDemandActive(ServerDemandActive::decode(src)?))
            }
            ShareControlPduType::ConfirmActivePdu => {
                Ok(ShareControlPdu::ClientConfirmActive(ClientConfirmActive::decode(src)?))
            }
            ShareControlPduType::DataPdu => Ok(ShareControlPdu::Data(ShareDataHeader::decode(src)?)),
            ShareControlPduType::DeactivateAllPdu => {
                Ok(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll::decode(src)?))
            }
            _ => Err(invalid_message_err!("share_type", "unexpected share control PDU type")),
        }
    }
}

impl PduEncode for ShareControlPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            ShareControlPdu::ServerDemandActive(pdu) => pdu.encode(dst),
            ShareControlPdu::ClientConfirmActive(pdu) => pdu.encode(dst),
            ShareControlPdu::Data(share_data_header) => share_data_header.encode(dst),
            ShareControlPdu::ServerDeactivateAll(deactivate_all) => deactivate_all.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            ShareControlPdu::ServerDemandActive(pdu) => pdu.size(),
            ShareControlPdu::ClientConfirmActive(pdu) => pdu.size(),
            ShareControlPdu::Data(share_data_header) => share_data_header.size(),
            ShareControlPdu::ServerDeactivateAll(deactivate_all) => deactivate_all.size(),
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

impl ShareDataHeader {
    const NAME: &'static str = "ShareDataHeader";

    const FIXED_PART_SIZE: usize = PADDING_FIELD_SIZE
        + STREAM_ID_FIELD_SIZE
        + UNCOMPRESSED_LENGTH_FIELD_SIZE
        + PDU_TYPE_FIELD_SIZE
        + COMPRESSION_TYPE_FIELD_SIZE
        + COMPRESSED_LENGTH_FIELD_SIZE;
}

impl PduEncode for ShareDataHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        if self.compression_flags.is_empty() {
            let compression_flags_with_type = self.compression_flags.bits() | self.compression_type.to_u8().unwrap();

            write_padding!(dst, 1);
            dst.write_u8(self.stream_priority.to_u8().unwrap());
            dst.write_u16(cast_length!(
                "uncompressedLength",
                self.share_data_pdu.size()
                    + PDU_TYPE_FIELD_SIZE
                    + COMPRESSION_TYPE_FIELD_SIZE
                    + COMPRESSED_LENGTH_FIELD_SIZE
            )?);
            dst.write_u8(self.share_data_pdu.share_header_type().to_u8().unwrap());
            dst.write_u8(compression_flags_with_type);
            dst.write_u16(0); // compressed length

            self.share_data_pdu.encode(dst)
        } else {
            Err(other_err!("Compression is not implemented"))
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.share_data_pdu.size()
    }
}

impl<'de> PduDecode<'de> for ShareDataHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_padding!(src, 1);
        let stream_priority = StreamPriority::from_u8(src.read_u8())
            .ok_or_else(|| invalid_message_err!("streamPriority", "Invalid stream priority"))?;
        let _uncompressed_length = src.read_u16();
        let pdu_type = ShareDataPduType::from_u8(src.read_u8())
            .ok_or_else(|| invalid_message_err!("pduType", "Invalid pdu type"))?;
        let compression_flags_with_type = src.read_u8();

        let compression_flags =
            CompressionFlags::from_bits_truncate(compression_flags_with_type & !SHARE_DATA_HEADER_COMPRESSION_MASK);
        let compression_type =
            client_info::CompressionType::from_u8(compression_flags_with_type & SHARE_DATA_HEADER_COMPRESSION_MASK)
                .ok_or_else(|| invalid_message_err!("compressionType", "Invalid compression type"))?;
        let _compressed_length = src.read_u16();

        let share_data_pdu = ShareDataPdu::from_type(src, pdu_type)?;

        Ok(Self {
            share_data_pdu,
            stream_priority,
            compression_flags,
            compression_type,
        })
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
    const NAME: &'static str = "ShareDataPdu";

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

    fn from_type(src: &mut ReadCursor<'_>, share_type: ShareDataPduType) -> PduResult<Self> {
        match share_type {
            ShareDataPduType::Synchronize => Ok(ShareDataPdu::Synchronize(SynchronizePdu::decode(src)?)),
            ShareDataPduType::Control => Ok(ShareDataPdu::Control(ControlPdu::decode(src)?)),
            ShareDataPduType::FontList => Ok(ShareDataPdu::FontList(FontPdu::decode(src)?)),
            ShareDataPduType::FontMap => Ok(ShareDataPdu::FontMap(FontPdu::decode(src)?)),
            ShareDataPduType::MonitorLayoutPdu => Ok(ShareDataPdu::MonitorLayout(MonitorLayoutPdu::decode(src)?)),
            ShareDataPduType::SaveSessionInfo => Ok(ShareDataPdu::SaveSessionInfo(SaveSessionInfoPdu::decode(src)?)),
            ShareDataPduType::FrameAcknowledgePdu => {
                Ok(ShareDataPdu::FrameAcknowledge(FrameAcknowledgePdu::decode(src)?))
            }
            ShareDataPduType::SetErrorInfoPdu => {
                Ok(ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu::decode(src)?))
            }
            ShareDataPduType::Input => Ok(ShareDataPdu::Input(InputEventPdu::decode(src)?)),
            ShareDataPduType::ShutdownRequest => Ok(ShareDataPdu::ShutdownRequest),
            ShareDataPduType::ShutdownDenied => Ok(ShareDataPdu::ShutdownDenied),
            ShareDataPduType::SuppressOutput => Ok(ShareDataPdu::SuppressOutput(SuppressOutputPdu::decode(src)?)),
            ShareDataPduType::RefreshRectangle => Ok(ShareDataPdu::RefreshRectangle(RefreshRectanglePdu::decode(src)?)),
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
            | ShareDataPduType::StatusInfoPdu => Err(other_err!("unsupported share data PDU")),
        }
    }
}

impl PduEncode for ShareDataPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            ShareDataPdu::Synchronize(pdu) => pdu.encode(dst),
            ShareDataPdu::Control(pdu) => pdu.encode(dst),
            ShareDataPdu::FontList(pdu) | ShareDataPdu::FontMap(pdu) => pdu.encode(dst),
            ShareDataPdu::MonitorLayout(pdu) => pdu.encode(dst),
            ShareDataPdu::SaveSessionInfo(pdu) => pdu.encode(dst),
            ShareDataPdu::FrameAcknowledge(pdu) => pdu.encode(dst),
            ShareDataPdu::ServerSetErrorInfo(pdu) => pdu.encode(dst),
            ShareDataPdu::Input(pdu) => pdu.encode(dst),
            ShareDataPdu::ShutdownRequest | ShareDataPdu::ShutdownDenied => Ok(()),
            ShareDataPdu::SuppressOutput(pdu) => pdu.encode(dst),
            ShareDataPdu::RefreshRectangle(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            ShareDataPdu::Synchronize(pdu) => pdu.size(),
            ShareDataPdu::Control(pdu) => pdu.size(),
            ShareDataPdu::FontList(pdu) | ShareDataPdu::FontMap(pdu) => pdu.size(),
            ShareDataPdu::MonitorLayout(pdu) => pdu.size(),
            ShareDataPdu::SaveSessionInfo(pdu) => pdu.size(),
            ShareDataPdu::FrameAcknowledge(pdu) => pdu.size(),
            ShareDataPdu::ServerSetErrorInfo(pdu) => pdu.size(),
            ShareDataPdu::Input(pdu) => pdu.size(),
            ShareDataPdu::ShutdownRequest | ShareDataPdu::ShutdownDenied => 0,
            ShareDataPdu::SuppressOutput(pdu) => pdu.size(),
            ShareDataPdu::RefreshRectangle(pdu) => pdu.size(),
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

/// 2.2.3.1 Server Deactivate All PDU
///
/// [2.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/8a29971a-df3c-48da-add2-8ed9a05edc89
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDeactivateAll;

impl PduDecode<'_> for ServerDeactivateAll {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let length_source_descriptor = src.read_u16();
        let _ = src.read_slice(length_source_descriptor.into());
        Ok(Self)
    }
}

impl PduEncode for ServerDeactivateAll {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        // A 16-bit, unsigned integer. The size in bytes of the sourceDescriptor field.
        dst.write_u16(1);
        // Variable number of bytes. The source descriptor. This field SHOULD be set to 0x00.
        dst.write_u8(0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Server Deactivate All"
    }

    fn size(&self) -> usize {
        2 /* length_source_descriptor */ + 1 /* source_descriptor */
    }
}
