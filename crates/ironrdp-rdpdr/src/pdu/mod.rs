use std::fmt;
use std::mem::size_of;

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{ensure_size, invalid_message_err, PduDecode, PduEncode, PduError, PduResult};

use self::efs::{
    ClientDeviceListAnnounce, ClientNameRequest, CoreCapability, CoreCapabilityKind, VersionAndIdPdu,
    VersionAndIdPduKind,
};

pub mod efs;

/// All available RDPDR PDUs.
pub enum RdpdrPdu {
    VersionAndIdPdu(VersionAndIdPdu),
    ClientNameRequest(ClientNameRequest),
    CoreCapability(CoreCapability),
    ClientDeviceListAnnounce(ClientDeviceListAnnounce),
    /// TODO: temporary value for development, this should be removed
    Unimplemented,
}

impl RdpdrPdu {
    /// Returns the [`SharedHeader`] of the PDU.
    fn header(&self) -> SharedHeader {
        match self {
            RdpdrPdu::VersionAndIdPdu(pdu) => match pdu.kind {
                VersionAndIdPduKind::ClientAnnounceReply => SharedHeader {
                    component: Component::RdpdrCtypCore,
                    packet_id: PacketId::CoreClientidConfirm,
                },
                VersionAndIdPduKind::ServerAnnounceRequest => SharedHeader {
                    component: Component::RdpdrCtypCore,
                    packet_id: PacketId::CoreServerAnnounce,
                },
                VersionAndIdPduKind::ServerClientIdConfirm => SharedHeader {
                    component: Component::RdpdrCtypCore,
                    packet_id: PacketId::CoreClientidConfirm,
                },
            },
            RdpdrPdu::ClientNameRequest(_) => SharedHeader {
                component: Component::RdpdrCtypCore,
                packet_id: PacketId::CoreClientName,
            },
            RdpdrPdu::CoreCapability(pdu) => match pdu.kind {
                CoreCapabilityKind::ServerCoreCapabilityRequest => SharedHeader {
                    component: Component::RdpdrCtypCore,
                    packet_id: PacketId::CoreServerCapability,
                },
                CoreCapabilityKind::ClientCoreCapabilityResponse => SharedHeader {
                    component: Component::RdpdrCtypCore,
                    packet_id: PacketId::CoreClientCapability,
                },
            },
            RdpdrPdu::ClientDeviceListAnnounce(_) => SharedHeader {
                component: Component::RdpdrCtypCore,
                packet_id: PacketId::CoreDevicelistAnnounce,
            },
            RdpdrPdu::Unimplemented => SharedHeader {
                component: Component::Unimplemented,
                packet_id: PacketId::Unimplemented,
            },
        }
    }
}

impl PduDecode<'_> for RdpdrPdu {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = SharedHeader::decode(src)?;
        match header.packet_id {
            PacketId::CoreServerAnnounce => Ok(RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu::decode(header, src)?)),
            PacketId::CoreServerCapability => Ok(RdpdrPdu::CoreCapability(CoreCapability::decode(header, src)?)),
            PacketId::CoreClientidConfirm => Ok(RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu::decode(header, src)?)),
            _ => Ok(RdpdrPdu::Unimplemented),
        }
    }
}

impl PduEncode for RdpdrPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.header().encode(dst)?;

        match self {
            RdpdrPdu::VersionAndIdPdu(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientNameRequest(pdu) => pdu.encode(dst),
            RdpdrPdu::CoreCapability(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientDeviceListAnnounce(pdu) => pdu.encode(dst),
            RdpdrPdu::Unimplemented => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            RdpdrPdu::VersionAndIdPdu(pdu) => pdu.name(),
            RdpdrPdu::ClientNameRequest(pdu) => pdu.name(),
            RdpdrPdu::CoreCapability(pdu) => pdu.name(),
            RdpdrPdu::ClientDeviceListAnnounce(pdu) => pdu.name(),
            RdpdrPdu::Unimplemented => "Unimplemented",
        }
    }

    fn size(&self) -> usize {
        SharedHeader::SIZE
            + match self {
                RdpdrPdu::VersionAndIdPdu(pdu) => pdu.size(),
                RdpdrPdu::ClientNameRequest(pdu) => pdu.size(),
                RdpdrPdu::CoreCapability(pdu) => pdu.size(),
                RdpdrPdu::ClientDeviceListAnnounce(pdu) => pdu.size(),
                RdpdrPdu::Unimplemented => 0,
            }
    }
}

impl fmt::Debug for RdpdrPdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionAndIdPdu(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::ClientNameRequest(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::CoreCapability(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::ClientDeviceListAnnounce(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::Unimplemented => {
                write!(f, "RdpdrPdu::Unimplemented")
            }
        }
    }
}

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

    fn encode(&self, dst: &mut WriteCursor) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::SIZE);
        dst.write_u16(self.component as u16);
        dst.write_u16(self.packet_id as u16);
        Ok(())
    }

    pub fn decode(src: &mut ReadCursor) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        Ok(Self {
            component: src.read_u16().try_into()?,
            packet_id: src.read_u16().try_into()?,
        })
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
pub enum Component {
    /// RDPDR_CTYP_CORE
    RdpdrCtypCore = 0x4472,
    /// RDPDR_CTYP_PRN
    RdpdrCtypPrn = 0x5052,
    /// TODO: temporary value for development, this should be removed
    Unimplemented,
}

impl TryFrom<u16> for Component {
    type Error = PduError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x4472 => Ok(Component::RdpdrCtypCore),
            0x5052 => Ok(Component::RdpdrCtypPrn),
            _ => Err(invalid_message_err!("try_from", "Component", "invalid value")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
pub enum PacketId {
    /// PAKID_CORE_SERVER_ANNOUNCE
    CoreServerAnnounce = 0x496E,
    /// PAKID_CORE_CLIENTID_CONFIRM
    CoreClientidConfirm = 0x4343,
    /// PAKID_CORE_CLIENT_NAME
    CoreClientName = 0x434E,
    /// PAKID_CORE_DEVICELIST_ANNOUNCE
    CoreDevicelistAnnounce = 0x4441,
    /// PAKID_CORE_DEVICE_REPLY
    CoreDeviceReply = 0x6472,
    /// PAKID_CORE_DEVICE_IOREQUEST
    CoreDeviceIorequest = 0x4952,
    /// PAKID_CORE_DEVICE_IOCOMPLETION
    CoreDeviceIocompletion = 0x4943,
    /// PAKID_CORE_SERVER_CAPABILITY
    CoreServerCapability = 0x5350,
    /// PAKID_CORE_CLIENT_CAPABILITY
    CoreClientCapability = 0x4350,
    /// PAKID_CORE_DEVICELIST_REMOVE
    CoreDevicelistRemove = 0x444D,
    /// PAKID_PRN_CACHE_DATA
    PrnCacheData = 0x5043,
    /// PAKID_CORE_USER_LOGGEDON
    CoreUserLoggedon = 0x554C,
    /// PAKID_PRN_USING_XPS
    PrnUsingXps = 0x5543,
    /// TODO: temporary value for development, this should be removed
    Unimplemented,
}

impl TryFrom<u16> for PacketId {
    type Error = PduError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x496E => Ok(PacketId::CoreServerAnnounce),
            0x4343 => Ok(PacketId::CoreClientidConfirm),
            0x434E => Ok(PacketId::CoreClientName),
            0x4441 => Ok(PacketId::CoreDevicelistAnnounce),
            0x6472 => Ok(PacketId::CoreDeviceReply),
            0x4952 => Ok(PacketId::CoreDeviceIorequest),
            0x4943 => Ok(PacketId::CoreDeviceIocompletion),
            0x5350 => Ok(PacketId::CoreServerCapability),
            0x4350 => Ok(PacketId::CoreClientCapability),
            0x444D => Ok(PacketId::CoreDevicelistRemove),
            0x5043 => Ok(PacketId::PrnCacheData),
            0x554C => Ok(PacketId::CoreUserLoggedon),
            0x5543 => Ok(PacketId::PrnUsingXps),
            _ => Err(invalid_message_err!("try_from", "PacketId", "invalid value")),
        }
    }
}
