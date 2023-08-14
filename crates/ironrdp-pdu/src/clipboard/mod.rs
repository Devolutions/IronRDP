//! This module implements RDP clipboard channel PDUs encode/decode logic as defined in
//! [MS-RDPECLIP]: Remote Desktop Protocol: Clipboard Virtual Channel Extension

mod capabilities;
mod client_temporary_directory;
mod file_contents;
mod format_data;
mod format_list;
mod lock;

pub use capabilities::*;
pub use client_temporary_directory::*;
pub use file_contents::*;
pub use format_data::*;
pub use format_list::*;
pub use lock::*;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{ensure_fixed_part_size, invalid_message_err, PduDecode, PduEncode, PduResult};
use bitflags::bitflags;

const MSG_TYPE_MONITOR_READY: u16 = 0x0001;
const MSG_TYPE_FORMAT_LIST: u16 = 0x0002;
const MSG_TYPE_FORMAT_LIST_RESPONSE: u16 = 0x0003;
const MSG_TYPE_FORMAT_DATA_REQUEST: u16 = 0x0004;
const MSG_TYPE_FORMAT_DATA_RESPONSE: u16 = 0x0005;
const MSG_TYPE_TEMPORARY_DIRECTORY: u16 = 0x0006;
const MSG_TYPE_CAPABILITIES: u16 = 0x0007;
const MSG_TYPE_FILE_CONTENTS_REQUEST: u16 = 0x0008;
const MSG_TYPE_FILE_CONTENTS_RESPONSE: u16 = 0x0009;
const MSG_TYPE_LOCK_CLIPDATA: u16 = 0x000A;
const MSG_TYPE_UNLOCK_CLIPDATA: u16 = 0x000B;

pub const FORMAT_ID_PALETTE: u32 = 9;
pub const FORMAT_ID_METAFILE: u32 = 3;
pub const FORMAT_NAME_FILE_LIST: &str = "FileGroupDescriptorW";

/// Header without message type included
struct PartialHeader {
    pub message_flags: ClipboardPduFlags,
    pub data_length: u32,
}

impl PartialHeader {
    const NAME: &str = "CLIPRDR_HEADER";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() + std::mem::size_of::<u32>();
    const SIZE: usize = Self::FIXED_PART_SIZE;

    pub fn new(inner_data_length: u32) -> Self {
        Self::new_with_flags(inner_data_length, ClipboardPduFlags::empty())
    }

    pub fn new_with_flags(data_length: u32, message_flags: ClipboardPduFlags) -> Self {
        Self {
            message_flags,
            data_length,
        }
    }

    pub fn data_length(&self) -> usize {
        self.data_length as usize
    }
}

impl<'de> PduDecode<'de> for PartialHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let message_flags = ClipboardPduFlags::from_bits_truncate(src.read_u16());
        let data_length = src.read_u32();

        Ok(Self {
            message_flags,
            data_length,
        })
    }
}

impl PduEncode for PartialHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.message_flags.bits());
        dst.write_u32(self.data_length);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// Clipboard channel message PDU
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPdu<'a> {
    MonitorReady,
    FormatList(FormatList<'a>),
    FormatListResponse(FormatListResponse),
    FormatDataRequest(FormatDataRequest),
    FormatDataResponse(FormatDataResponse<'a>),
    TemporaryDirectory(ClientTemporaryDirectory<'a>),
    Capabilites(Capabilities),
    FileContentsRequest(FileContentsRequest),
    FileContentsResponse(FileContentsResponse<'a>),
    LockData(LockDataId),
    UnlockData(LockDataId),
}

impl ClipboardPdu<'_> {
    const NAME: &str = "CliboardPdu";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>();
}

impl PduEncode for ClipboardPdu<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let write_empty_pdu = |dst: &mut WriteCursor<'_>| {
            let header = PartialHeader::new(0);
            header.encode(dst)
        };

        match self {
            ClipboardPdu::MonitorReady => {
                dst.write_u16(MSG_TYPE_MONITOR_READY);
                write_empty_pdu(dst)
            }
            ClipboardPdu::FormatList(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_LIST);
                pdu.encode(dst)
            }
            ClipboardPdu::FormatListResponse(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_LIST_RESPONSE);
                pdu.encode(dst)
            }
            ClipboardPdu::FormatDataRequest(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_DATA_REQUEST);
                pdu.encode(dst)
            }
            ClipboardPdu::FormatDataResponse(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_DATA_RESPONSE);
                pdu.encode(dst)
            }
            ClipboardPdu::TemporaryDirectory(pdu) => {
                dst.write_u16(MSG_TYPE_TEMPORARY_DIRECTORY);
                pdu.encode(dst)
            }
            ClipboardPdu::Capabilites(pdu) => {
                dst.write_u16(MSG_TYPE_CAPABILITIES);
                pdu.encode(dst)
            }
            ClipboardPdu::FileContentsRequest(pdu) => {
                dst.write_u16(MSG_TYPE_FILE_CONTENTS_REQUEST);
                pdu.encode(dst)
            }
            ClipboardPdu::FileContentsResponse(pdu) => {
                dst.write_u16(MSG_TYPE_FILE_CONTENTS_RESPONSE);
                pdu.encode(dst)
            }
            ClipboardPdu::LockData(pdu) => {
                dst.write_u16(MSG_TYPE_LOCK_CLIPDATA);
                pdu.encode(dst)
            }
            ClipboardPdu::UnlockData(pdu) => {
                dst.write_u16(MSG_TYPE_UNLOCK_CLIPDATA);
                pdu.encode(dst)
            }
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let empty_size = PartialHeader::SIZE;

        let variable_size = match self {
            ClipboardPdu::MonitorReady => empty_size,
            ClipboardPdu::FormatList(pdu) => pdu.size(),
            ClipboardPdu::FormatListResponse(pdu) => pdu.size(),
            ClipboardPdu::FormatDataRequest(pdu) => pdu.size(),
            ClipboardPdu::FormatDataResponse(pdu) => pdu.size(),
            ClipboardPdu::TemporaryDirectory(pdu) => pdu.size(),
            ClipboardPdu::Capabilites(pdu) => pdu.size(),
            ClipboardPdu::FileContentsRequest(pdu) => pdu.size(),
            ClipboardPdu::FileContentsResponse(pdu) => pdu.size(),
            ClipboardPdu::LockData(pdu) => pdu.size(),
            ClipboardPdu::UnlockData(pdu) => pdu.size(),
        };

        Self::FIXED_PART_SIZE + variable_size
    }
}

impl<'de> PduDecode<'de> for ClipboardPdu<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let read_empty_pdu = |src: &mut ReadCursor<'de>| {
            let _header = PartialHeader::decode(src)?;
            Ok(())
        };

        let pdu = match src.read_u16() {
            MSG_TYPE_MONITOR_READY => {
                read_empty_pdu(src)?;
                ClipboardPdu::MonitorReady
            }
            MSG_TYPE_FORMAT_LIST => ClipboardPdu::FormatList(FormatList::decode(src)?),
            MSG_TYPE_FORMAT_LIST_RESPONSE => ClipboardPdu::FormatListResponse(FormatListResponse::decode(src)?),
            MSG_TYPE_FORMAT_DATA_REQUEST => ClipboardPdu::FormatDataRequest(FormatDataRequest::decode(src)?),
            MSG_TYPE_FORMAT_DATA_RESPONSE => ClipboardPdu::FormatDataResponse(FormatDataResponse::decode(src)?),
            MSG_TYPE_TEMPORARY_DIRECTORY => ClipboardPdu::TemporaryDirectory(ClientTemporaryDirectory::decode(src)?),
            MSG_TYPE_CAPABILITIES => ClipboardPdu::Capabilites(Capabilities::decode(src)?),
            MSG_TYPE_FILE_CONTENTS_REQUEST => ClipboardPdu::FileContentsRequest(FileContentsRequest::decode(src)?),
            MSG_TYPE_FILE_CONTENTS_RESPONSE => ClipboardPdu::FileContentsResponse(FileContentsResponse::decode(src)?),
            MSG_TYPE_LOCK_CLIPDATA => ClipboardPdu::LockData(LockDataId::decode(src)?),
            MSG_TYPE_UNLOCK_CLIPDATA => ClipboardPdu::UnlockData(LockDataId::decode(src)?),
            _ => return Err(invalid_message_err!("msgType", "Unknown clipboard PDU type")),
        };

        Ok(pdu)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    /// Represents `msgFlags` field of `CLIPRDR_HEADER` structure
    pub struct ClipboardPduFlags: u16 {
        /// Used by the Format List Response PDU, Format Data Response PDU, and File
        /// Contents Response PDU to indicate that the associated request Format List PDU,
        /// Format Data Request PDU, and File Contents Request PDU were processed
        /// successfully
        const RESPONSE_OK = 0x0001;
        /// Used by the Format List Response PDU, Format Data Response PDU, and File
        /// Contents Response PDU to indicate that the associated Format List PDU, Format
        /// Data Request PDU, and File Contents Request PDU were not processed successfull
        const RESPONSE_FAIL = 0x0002;
        /// Used by the Short Format Name variant of the Format List Response PDU to indicate
        /// that the format names are in ASCII 8
        const ASCII_NAMES = 0x0004;
    }
}
