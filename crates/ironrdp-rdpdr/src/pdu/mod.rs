use core::fmt::{self, Display};
use core::mem::size_of;

use ironrdp_core::{
    ensure_size, invalid_field_err, unsupported_value_err, DecodeError, DecodeResult, EncodeResult, ReadCursor,
    WriteCursor,
};
use ironrdp_core::{Decode, Encode};
use ironrdp_svc::SvcEncode;

use self::efs::{
    ClientDeviceListAnnounce, ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse,
    ClientDriveQueryVolumeInformationResponse, ClientDriveSetInformationResponse, ClientNameRequest, CoreCapability,
    CoreCapabilityKind, DeviceCloseResponse, DeviceControlResponse, DeviceCreateResponse, DeviceIoRequest,
    DeviceReadResponse, DeviceWriteResponse, ServerDeviceAnnounceResponse, VersionAndIdPdu, VersionAndIdPduKind,
};

pub mod efs;
pub mod esc;

/// All available RDPDR PDUs.
pub enum RdpdrPdu {
    VersionAndIdPdu(VersionAndIdPdu),
    ClientNameRequest(ClientNameRequest),
    CoreCapability(CoreCapability),
    ClientDeviceListAnnounce(ClientDeviceListAnnounce),
    ServerDeviceAnnounceResponse(ServerDeviceAnnounceResponse),
    DeviceIoRequest(DeviceIoRequest),
    DeviceControlResponse(DeviceControlResponse),
    DeviceCreateResponse(DeviceCreateResponse),
    ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse),
    DeviceCloseResponse(DeviceCloseResponse),
    ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse),
    ClientDriveQueryVolumeInformationResponse(ClientDriveQueryVolumeInformationResponse),
    DeviceReadResponse(DeviceReadResponse),
    DeviceWriteResponse(DeviceWriteResponse),
    ClientDriveSetInformationResponse(ClientDriveSetInformationResponse),
    EmptyResponse,
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
            RdpdrPdu::ServerDeviceAnnounceResponse(_) => SharedHeader {
                component: Component::RdpdrCtypCore,
                packet_id: PacketId::CoreDeviceReply,
            },
            RdpdrPdu::DeviceIoRequest(_) => SharedHeader {
                component: Component::RdpdrCtypCore,
                packet_id: PacketId::CoreDeviceIoRequest,
            },
            RdpdrPdu::DeviceControlResponse(_)
            | RdpdrPdu::DeviceCreateResponse(_)
            | RdpdrPdu::ClientDriveQueryInformationResponse(_)
            | RdpdrPdu::DeviceCloseResponse(_)
            | RdpdrPdu::ClientDriveQueryDirectoryResponse(_)
            | RdpdrPdu::ClientDriveQueryVolumeInformationResponse(_)
            | RdpdrPdu::DeviceReadResponse(_)
            | RdpdrPdu::DeviceWriteResponse(_)
            | RdpdrPdu::ClientDriveSetInformationResponse(_)
            | RdpdrPdu::EmptyResponse => SharedHeader {
                component: Component::RdpdrCtypCore,
                packet_id: PacketId::CoreDeviceIoCompletion,
            },
        }
    }
}

impl Decode<'_> for RdpdrPdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedHeader::decode(src)?;
        match header.packet_id {
            PacketId::CoreServerAnnounce => Ok(RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu::decode(header, src)?)),
            PacketId::CoreServerCapability => Ok(RdpdrPdu::CoreCapability(CoreCapability::decode(header, src)?)),
            PacketId::CoreClientidConfirm => Ok(RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu::decode(header, src)?)),
            PacketId::CoreDeviceReply => Ok(RdpdrPdu::ServerDeviceAnnounceResponse(
                ServerDeviceAnnounceResponse::decode(src)?,
            )),
            PacketId::CoreDeviceIoRequest => Ok(RdpdrPdu::DeviceIoRequest(DeviceIoRequest::decode(src)?)),
            _ => Err(unsupported_value_err!(
                "RdpdrPdu",
                "PacketId",
                header.packet_id.to_string()
            )),
        }
    }
}

impl Encode for RdpdrPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.header().encode(dst)?;

        match self {
            RdpdrPdu::VersionAndIdPdu(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientNameRequest(pdu) => pdu.encode(dst),
            RdpdrPdu::CoreCapability(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientDeviceListAnnounce(pdu) => pdu.encode(dst),
            RdpdrPdu::ServerDeviceAnnounceResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::DeviceIoRequest(pdu) => pdu.encode(dst),
            RdpdrPdu::DeviceControlResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::DeviceCreateResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientDriveQueryInformationResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::DeviceCloseResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientDriveQueryDirectoryResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientDriveQueryVolumeInformationResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::DeviceReadResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::DeviceWriteResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::ClientDriveSetInformationResponse(pdu) => pdu.encode(dst),
            RdpdrPdu::EmptyResponse => {
                // https://github.com/FreeRDP/FreeRDP/blob/dfa231c0a55b005af775b833f92f6bcd30363d77/channels/drive/client/drive_main.c#L601
                dst.write_u32(0);
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            RdpdrPdu::VersionAndIdPdu(pdu) => pdu.name(),
            RdpdrPdu::ClientNameRequest(pdu) => pdu.name(),
            RdpdrPdu::CoreCapability(pdu) => pdu.name(),
            RdpdrPdu::ClientDeviceListAnnounce(pdu) => pdu.name(),
            RdpdrPdu::ServerDeviceAnnounceResponse(pdu) => pdu.name(),
            RdpdrPdu::DeviceIoRequest(pdu) => pdu.name(),
            RdpdrPdu::DeviceControlResponse(pdu) => pdu.name(),
            RdpdrPdu::DeviceCreateResponse(pdu) => pdu.name(),
            RdpdrPdu::ClientDriveQueryInformationResponse(pdu) => pdu.name(),
            RdpdrPdu::DeviceCloseResponse(pdu) => pdu.name(),
            RdpdrPdu::ClientDriveQueryDirectoryResponse(pdu) => pdu.name(),
            RdpdrPdu::ClientDriveQueryVolumeInformationResponse(pdu) => pdu.name(),
            RdpdrPdu::DeviceReadResponse(pdu) => pdu.name(),
            RdpdrPdu::DeviceWriteResponse(pdu) => pdu.name(),
            RdpdrPdu::ClientDriveSetInformationResponse(pdu) => pdu.name(),
            RdpdrPdu::EmptyResponse => "EmptyResponse",
        }
    }

    fn size(&self) -> usize {
        SharedHeader::SIZE
            + match self {
                RdpdrPdu::VersionAndIdPdu(pdu) => pdu.size(),
                RdpdrPdu::ClientNameRequest(pdu) => pdu.size(),
                RdpdrPdu::CoreCapability(pdu) => pdu.size(),
                RdpdrPdu::ClientDeviceListAnnounce(pdu) => pdu.size(),
                RdpdrPdu::ServerDeviceAnnounceResponse(pdu) => pdu.size(),
                RdpdrPdu::DeviceIoRequest(pdu) => pdu.size(),
                RdpdrPdu::DeviceControlResponse(pdu) => pdu.size(),
                RdpdrPdu::DeviceCreateResponse(pdu) => pdu.size(),
                RdpdrPdu::ClientDriveQueryInformationResponse(pdu) => pdu.size(),
                RdpdrPdu::DeviceCloseResponse(pdu) => pdu.size(),
                RdpdrPdu::ClientDriveQueryDirectoryResponse(pdu) => pdu.size(),
                RdpdrPdu::ClientDriveQueryVolumeInformationResponse(pdu) => pdu.size(),
                RdpdrPdu::DeviceReadResponse(pdu) => pdu.size(),
                RdpdrPdu::DeviceWriteResponse(pdu) => pdu.size(),
                RdpdrPdu::ClientDriveSetInformationResponse(pdu) => pdu.size(),
                RdpdrPdu::EmptyResponse => size_of::<u32>(),
            }
    }
}

impl SvcEncode for RdpdrPdu {}

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
            Self::ServerDeviceAnnounceResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::DeviceIoRequest(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::DeviceControlResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::DeviceCreateResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::ClientDriveQueryInformationResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::DeviceCloseResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::ClientDriveQueryDirectoryResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::ClientDriveQueryVolumeInformationResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::DeviceReadResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::DeviceWriteResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::ClientDriveSetInformationResponse(it) => {
                write!(f, "RdpdrPdu({:?})", it)
            }
            Self::EmptyResponse => {
                write!(f, "RdpdrPdu(EmptyResponse)")
            }
        }
    }
}

impl From<DeviceControlResponse> for RdpdrPdu {
    fn from(value: DeviceControlResponse) -> Self {
        Self::DeviceControlResponse(value)
    }
}

impl From<DeviceCreateResponse> for RdpdrPdu {
    fn from(value: DeviceCreateResponse) -> Self {
        Self::DeviceCreateResponse(value)
    }
}

impl From<ClientDriveQueryInformationResponse> for RdpdrPdu {
    fn from(value: ClientDriveQueryInformationResponse) -> Self {
        Self::ClientDriveQueryInformationResponse(value)
    }
}

impl From<DeviceCloseResponse> for RdpdrPdu {
    fn from(value: DeviceCloseResponse) -> Self {
        Self::DeviceCloseResponse(value)
    }
}

impl From<ClientDriveQueryDirectoryResponse> for RdpdrPdu {
    fn from(value: ClientDriveQueryDirectoryResponse) -> Self {
        Self::ClientDriveQueryDirectoryResponse(value)
    }
}

impl From<ClientDriveQueryVolumeInformationResponse> for RdpdrPdu {
    fn from(value: ClientDriveQueryVolumeInformationResponse) -> Self {
        Self::ClientDriveQueryVolumeInformationResponse(value)
    }
}

impl From<DeviceReadResponse> for RdpdrPdu {
    fn from(value: DeviceReadResponse) -> Self {
        Self::DeviceReadResponse(value)
    }
}

impl From<DeviceWriteResponse> for RdpdrPdu {
    fn from(value: DeviceWriteResponse) -> Self {
        Self::DeviceWriteResponse(value)
    }
}

impl From<ClientDriveSetInformationResponse> for RdpdrPdu {
    fn from(value: ClientDriveSetInformationResponse) -> Self {
        Self::ClientDriveSetInformationResponse(value)
    }
}

/// [2.2.1.1] Shared Header (RDPDR_HEADER), a header that is shared by all RDPDR PDUs.
///
/// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/29d4108f-8163-4a67-8271-e48c4b9c2a7c
#[derive(Debug)]
pub struct SharedHeader {
    pub component: Component,
    pub packet_id: PacketId,
}

impl SharedHeader {
    const SIZE: usize = size_of::<u16>() * 2;

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: Self::SIZE);
        dst.write_u16(self.component.into());
        dst.write_u16(self.packet_id.into());
        Ok(())
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
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
}

impl TryFrom<u16> for Component {
    type Error = DecodeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x4472 => Ok(Component::RdpdrCtypCore),
            0x5052 => Ok(Component::RdpdrCtypPrn),
            _ => Err(invalid_field_err!("try_from", "Component", "invalid value")),
        }
    }
}

impl From<Component> for u16 {
    fn from(component: Component) -> Self {
        component as u16
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
    CoreDeviceIoRequest = 0x4952,
    /// PAKID_CORE_DEVICE_IOCOMPLETION
    CoreDeviceIoCompletion = 0x4943,
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
}

impl TryFrom<u16> for PacketId {
    type Error = DecodeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x496E => Ok(PacketId::CoreServerAnnounce),
            0x4343 => Ok(PacketId::CoreClientidConfirm),
            0x434E => Ok(PacketId::CoreClientName),
            0x4441 => Ok(PacketId::CoreDevicelistAnnounce),
            0x6472 => Ok(PacketId::CoreDeviceReply),
            0x4952 => Ok(PacketId::CoreDeviceIoRequest),
            0x4943 => Ok(PacketId::CoreDeviceIoCompletion),
            0x5350 => Ok(PacketId::CoreServerCapability),
            0x4350 => Ok(PacketId::CoreClientCapability),
            0x444D => Ok(PacketId::CoreDevicelistRemove),
            0x5043 => Ok(PacketId::PrnCacheData),
            0x554C => Ok(PacketId::CoreUserLoggedon),
            0x5543 => Ok(PacketId::PrnUsingXps),
            _ => Err(invalid_field_err!("try_from", "PacketId", "invalid value")),
        }
    }
}

impl Display for PacketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacketId::CoreServerAnnounce => write!(f, "PAKID_CORE_SERVER_ANNOUNCE"),
            PacketId::CoreClientidConfirm => write!(f, "PAKID_CORE_CLIENTID_CONFIRM"),
            PacketId::CoreClientName => write!(f, "PAKID_CORE_CLIENT_NAME"),
            PacketId::CoreDevicelistAnnounce => write!(f, "PAKID_CORE_DEVICELIST_ANNOUNCE"),
            PacketId::CoreDeviceReply => write!(f, "PAKID_CORE_DEVICE_REPLY"),
            PacketId::CoreDeviceIoRequest => write!(f, "PAKID_CORE_DEVICE_IOREQUEST"),
            PacketId::CoreDeviceIoCompletion => write!(f, "PAKID_CORE_DEVICE_IOCOMPLETION"),
            PacketId::CoreServerCapability => write!(f, "PAKID_CORE_SERVER_CAPABILITY"),
            PacketId::CoreClientCapability => write!(f, "PAKID_CORE_CLIENT_CAPABILITY"),
            PacketId::CoreDevicelistRemove => write!(f, "PAKID_CORE_DEVICELIST_REMOVE"),
            PacketId::PrnCacheData => write!(f, "PAKID_PRN_CACHE_DATA"),
            PacketId::CoreUserLoggedon => write!(f, "PAKID_CORE_USER_LOGGEDON"),
            PacketId::PrnUsingXps => write!(f, "PAKID_PRN_USING_XPS"),
        }
    }
}

impl From<PacketId> for u16 {
    fn from(packet_id: PacketId) -> Self {
        packet_id as u16
    }
}
