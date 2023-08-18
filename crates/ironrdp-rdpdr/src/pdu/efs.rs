//! PDUs for [[MS-RDPEFS]: Remote Desktop Protocol: File System Virtual Channel Extension](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5)

use std::fmt::Debug;

use ironrdp_pdu::utils::{encoded_str_len, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{
    cursor::{ReadCursor, WriteCursor},
    PduEncode, PduResult,
};
use ironrdp_pdu::{invalid_message_err, unexpected_message_type_err, PduError};

/// [2.2.1.1 Shared Header (RDPDR_HEADER)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/29d4108f-8163-4a67-8271-e48c4b9c2a7c)
/// A header that is shared by all RDPDR PDUs.
#[derive(Debug)]
pub struct SharedHeader {
    pub component: Component, // u16
    pub packet_id: PacketId,  // u16
}

impl SharedHeader {
    pub fn decode(src: &mut ReadCursor) -> PduResult<Self> {
        Ok(Self {
            component: src.read_u16().try_into()?,
            packet_id: src.read_u16().try_into()?,
        })
    }

    fn encode(&self, dst: &mut WriteCursor) -> PduResult<()> {
        dst.write_u16(self.component as u16);
        dst.write_u16(self.packet_id as u16);
        Ok(())
    }

    fn size(&self) -> usize {
        4 // 2 * u16  = 2 * 2 bytes = 4 bytes
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum Component {
    RDPDR_CTYP_CORE = 0x4472,
    RDPDR_CTYP_PRN = 0x5052,
}

impl TryFrom<u16> for Component {
    type Error = PduError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x4472 => Ok(Component::RDPDR_CTYP_CORE),
            0x5052 => Ok(Component::RDPDR_CTYP_PRN),
            _ => Err(invalid_message_err!("try_from", "Component", "invalid value")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum PacketId {
    PAKID_CORE_SERVER_ANNOUNCE = 0x496E,
    PAKID_CORE_CLIENTID_CONFIRM = 0x4343,
    PAKID_CORE_CLIENT_NAME = 0x434E,
    PAKID_CORE_DEVICELIST_ANNOUNCE = 0x4441,
    PAKID_CORE_DEVICE_REPLY = 0x6472,
    PAKID_CORE_DEVICE_IOREQUEST = 0x4952,
    PAKID_CORE_DEVICE_IOCOMPLETION = 0x4943,
    PAKID_CORE_SERVER_CAPABILITY = 0x5350,
    PAKID_CORE_CLIENT_CAPABILITY = 0x4350,
    PAKID_CORE_DEVICELIST_REMOVE = 0x444D,
    PAKID_PRN_CACHE_DATA = 0x5043,
    PAKID_CORE_USER_LOGGEDON = 0x554C,
    PAKID_PRN_USING_XPS = 0x5543,
}

impl std::convert::TryFrom<u16> for PacketId {
    type Error = PduError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x496E => Ok(PacketId::PAKID_CORE_SERVER_ANNOUNCE),
            0x4343 => Ok(PacketId::PAKID_CORE_CLIENTID_CONFIRM),
            0x434E => Ok(PacketId::PAKID_CORE_CLIENT_NAME),
            0x4441 => Ok(PacketId::PAKID_CORE_DEVICELIST_ANNOUNCE),
            0x6472 => Ok(PacketId::PAKID_CORE_DEVICE_REPLY),
            0x4952 => Ok(PacketId::PAKID_CORE_DEVICE_IOREQUEST),
            0x4943 => Ok(PacketId::PAKID_CORE_DEVICE_IOCOMPLETION),
            0x5350 => Ok(PacketId::PAKID_CORE_SERVER_CAPABILITY),
            0x4350 => Ok(PacketId::PAKID_CORE_CLIENT_CAPABILITY),
            0x444D => Ok(PacketId::PAKID_CORE_DEVICELIST_REMOVE),
            0x5043 => Ok(PacketId::PAKID_PRN_CACHE_DATA),
            0x554C => Ok(PacketId::PAKID_CORE_USER_LOGGEDON),
            0x5543 => Ok(PacketId::PAKID_PRN_USING_XPS),
            _ => Err(invalid_message_err!("try_from", "PacketId", "invalid value")),
        }
    }
}

#[derive(Debug)]
pub enum VersionAndIdPDUKind {
    /// [2.2.2.2 Server Announce Request (DR_CORE_SERVER_ANNOUNCE_REQ)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/046047aa-62d8-49f9-bf16-7fe41880aaf4)
    ServerAnnounceRequest,
    /// [2.2.2.3 Client Announce Reply (DR_CORE_CLIENT_ANNOUNCE_RSP)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d6fe6d1b-c145-4a6f-99aa-4fe3cdcea398)
    ClientAnnounceReply,
}

#[derive(Debug)]
/// VersionAndIdPDU is a fixed size structure representing multiple PDUs.
/// The kind field is used to determine the actual PDU type.
pub struct VersionAndIdPDU {
    version_major: u16,
    version_minor: u16,
    pub client_id: u32,
    kind: VersionAndIdPDUKind,
}

impl VersionAndIdPDU {
    pub fn new(version_major: u16, version_minor: u16, client_id: u32, kind: VersionAndIdPDUKind) -> Self {
        Self {
            version_major,
            version_minor,
            client_id,
            kind,
        }
    }

    fn header(&self) -> PduResult<SharedHeader> {
        match self.kind {
            VersionAndIdPDUKind::ClientAnnounceReply => Ok(SharedHeader {
                component: Component::RDPDR_CTYP_CORE,
                packet_id: PacketId::PAKID_CORE_CLIENTID_CONFIRM,
            }),
            VersionAndIdPDUKind::ServerAnnounceRequest => Err(unexpected_message_type_err!("ServerAnnounceRequest", 0)),
        }
    }

    pub fn decode(src: &mut ReadCursor, kind: VersionAndIdPDUKind) -> PduResult<Self> {
        Ok(Self {
            version_major: src.read_u16(),
            version_minor: src.read_u16(),
            client_id: src.read_u32(),
            kind,
        })
    }
}

impl PduEncode for VersionAndIdPDU {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.header()?.encode(dst)?;
        dst.write_u16(self.version_major);
        dst.write_u16(self.version_minor);
        dst.write_u32(self.client_id);
        Ok(())
    }

    fn name(&self) -> &'static str {
        match self.kind {
            VersionAndIdPDUKind::ServerAnnounceRequest => "ServerAnnounceRequest",
            VersionAndIdPDUKind::ClientAnnounceReply => "ClientAnnounceReply",
        }
    }

    fn size(&self) -> usize {
        // header bytes + (2 * u16) + u32 = header bytes + (2 * 2 bytes) + 4 bytes = header bytes + 8 bytes
        self.header().unwrap().size() + 8
    }
}

/// 2.2.2.4 Client Name Request (DR_CORE_CLIENT_NAME_REQ)
/// https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/902497f1-3b1c-4aee-95f8-1668f9b7b7d2
#[derive(Debug)]
pub struct ClientNameRequest {
    unicode_flag: ClientNameRequestUnicodeFlag,
    computer_name: String,
}

impl ClientNameRequest {
    pub fn new(computer_name: String) -> Self {
        Self {
            unicode_flag: ClientNameRequestUnicodeFlag::Unicode,
            computer_name,
        }
    }

    fn header(&self) -> SharedHeader {
        SharedHeader {
            component: Component::RDPDR_CTYP_CORE,
            packet_id: PacketId::PAKID_CORE_CLIENT_NAME,
        }
    }
}

impl PduEncode for ClientNameRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.header().encode(dst)?;
        dst.write_u32(self.unicode_flag as u32);
        dst.write_u32(0); // // CodePage (4 bytes): it MUST be set to 0
        dst.write_u32(encoded_str_len(&self.computer_name, self.unicode_flag.into(), true) as u32);
        write_string_to_cursor(dst, &self.computer_name, self.unicode_flag.into(), true)
    }

    fn name(&self) -> &'static str {
        "ClientNameRequest"
    }

    fn size(&self) -> usize {
        // header bytes + u32 * 3 + computer_name_len = header bytes + 4 bytes * 3 + computer_name_len = header bytes + 12 + computer_name_len
        self.header().size() + 12 + encoded_str_len(&self.computer_name, self.unicode_flag.into(), true)
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
enum ClientNameRequestUnicodeFlag {
    #[allow(dead_code)]
    Ascii = 0x0,
    Unicode = 0x1,
}

impl From<ClientNameRequestUnicodeFlag> for CharacterSet {
    fn from(val: ClientNameRequestUnicodeFlag) -> Self {
        match val {
            ClientNameRequestUnicodeFlag::Ascii => CharacterSet::Ansi,
            ClientNameRequestUnicodeFlag::Unicode => CharacterSet::Unicode,
        }
    }
}
