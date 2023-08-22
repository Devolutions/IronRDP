//! PDUs for [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]
//!
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

use std::fmt::Debug;
use std::mem::size_of;

use ironrdp_pdu::utils::{encoded_str_len, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{
    cursor::{ReadCursor, WriteCursor},
    PduEncode, PduResult,
};
use ironrdp_pdu::{ensure_size, invalid_message_err, PduError};

/// [2.2.1.1 Shared Header (RDPDR_HEADER)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/29d4108f-8163-4a67-8271-e48c4b9c2a7c)
/// A header that is shared by all RDPDR PDUs.
#[derive(Debug)]
pub struct SharedHeader {
    pub component: Component,
    pub packet_id: PacketId,
}

impl SharedHeader {
    const NAME: &str = "RDPDR_HEADER";
    const SIZE: usize = size_of::<u16>() * 2;

    pub fn decode(src: &mut ReadCursor) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        Ok(Self {
            component: src.read_u16().try_into()?,
            packet_id: src.read_u16().try_into()?,
        })
    }

    fn encode(&self, dst: &mut WriteCursor) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::SIZE);
        dst.write_u16(self.component as u16);
        dst.write_u16(self.packet_id as u16);
        Ok(())
    }

    fn size(&self) -> usize {
        Self::SIZE
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
pub enum VersionAndIdPduKind {
    /// [2.2.2.2 Server Announce Request (DR_CORE_SERVER_ANNOUNCE_REQ)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/046047aa-62d8-49f9-bf16-7fe41880aaf4)
    ServerAnnounceRequest,
    /// [2.2.2.3 Client Announce Reply (DR_CORE_CLIENT_ANNOUNCE_RSP)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d6fe6d1b-c145-4a6f-99aa-4fe3cdcea398)
    ClientAnnounceReply,
}

impl VersionAndIdPduKind {
    fn name(&self) -> &'static str {
        match self {
            VersionAndIdPduKind::ServerAnnounceRequest => "ServerAnnounceRequest",
            VersionAndIdPduKind::ClientAnnounceReply => "ClientAnnounceReply",
        }
    }
}

/// VersionAndIdPDU is a fixed size structure representing multiple PDUs.
///
/// The kind field is used to determine the actual PDU type.
#[derive(Debug)]
pub struct VersionAndIdPdu {
    pub version_major: u16,
    pub version_minor: u16,
    pub client_id: u32,
    pub kind: VersionAndIdPduKind,
}

impl VersionAndIdPdu {
    /// The size of the PDU without the header
    const HEADERLESS_SIZE: usize = (size_of::<u16>() * 2) + size_of::<u32>();

    fn header(&self) -> SharedHeader {
        match self.kind {
            VersionAndIdPduKind::ClientAnnounceReply => SharedHeader {
                component: Component::RDPDR_CTYP_CORE,
                packet_id: PacketId::PAKID_CORE_CLIENTID_CONFIRM,
            },
            VersionAndIdPduKind::ServerAnnounceRequest => SharedHeader {
                component: Component::RDPDR_CTYP_CORE,
                packet_id: PacketId::PAKID_CORE_SERVER_ANNOUNCE,
            },
        }
    }

    pub fn decode(src: &mut ReadCursor, kind: VersionAndIdPduKind) -> PduResult<Self> {
        ensure_size!(ctx: kind.name(), in: src, size: Self::HEADERLESS_SIZE);
        Ok(Self {
            version_major: src.read_u16(),
            version_minor: src.read_u16(),
            client_id: src.read_u32(),
            kind,
        })
    }
}

impl PduEncode for VersionAndIdPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(ctx: self.name(), in: dst, size: self.size());
        self.header().encode(dst)?;
        dst.write_u16(self.version_major);
        dst.write_u16(self.version_minor);
        dst.write_u32(self.client_id);
        Ok(())
    }

    fn name(&self) -> &'static str {
        self.kind.name()
    }

    fn size(&self) -> usize {
        Self::HEADERLESS_SIZE + self.header().size()
    }
}

/// [2.2.2.4 Client Name Request (DR_CORE_CLIENT_NAME_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/902497f1-3b1c-4aee-95f8-1668f9b7b7d2)
#[derive(Debug)]
pub enum ClientNameRequest {
    Ascii(String),
    Unicode(String),
}

impl ClientNameRequest {
    const NAME: &str = "ClientNameRequest";
    pub fn new(computer_name: String, kind: ClientNameRequestUnicodeFlag) -> Self {
        match kind {
            ClientNameRequestUnicodeFlag::Ascii => ClientNameRequest::Ascii(computer_name),
            ClientNameRequestUnicodeFlag::Unicode => ClientNameRequest::Unicode(computer_name),
        }
    }
    fn header(&self) -> SharedHeader {
        SharedHeader {
            component: Component::RDPDR_CTYP_CORE,
            packet_id: PacketId::PAKID_CORE_CLIENT_NAME,
        }
    }

    fn unicode_flag(&self) -> ClientNameRequestUnicodeFlag {
        match self {
            ClientNameRequest::Ascii(_) => ClientNameRequestUnicodeFlag::Ascii,
            ClientNameRequest::Unicode(_) => ClientNameRequestUnicodeFlag::Unicode,
        }
    }

    fn computer_name(&self) -> &str {
        match self {
            ClientNameRequest::Ascii(name) => name,
            ClientNameRequest::Unicode(name) => name,
        }
    }
}

impl PduEncode for ClientNameRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header().encode(dst)?;
        dst.write_u32(self.unicode_flag() as u32);
        dst.write_u32(0); // // CodePage (4 bytes): it MUST be set to 0
        dst.write_u32(encoded_str_len(self.computer_name(), self.unicode_flag().into(), true) as u32);
        write_string_to_cursor(dst, self.computer_name(), self.unicode_flag().into(), true)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.header().size()
            + (size_of::<u32>() * 3) // unicode_flag + CodePage + ComputerNameLen
            + encoded_str_len(self.computer_name(), self.unicode_flag().into(), true)
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum ClientNameRequestUnicodeFlag {
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
