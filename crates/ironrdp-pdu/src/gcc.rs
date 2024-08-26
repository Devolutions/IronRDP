use std::io;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use thiserror::Error;

use crate::{decode, PduDecode, PduEncode, PduError, PduErrorKind, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

pub mod conference_create;

mod cluster_data;
mod core_data;
mod message_channel_data;
mod monitor_data;
mod monitor_extended_data;
mod multi_transport_channel_data;
mod network_data;
mod security_data;

pub use self::cluster_data::{ClientClusterData, ClusterDataError, RedirectionFlags, RedirectionVersion};
pub use self::conference_create::{ConferenceCreateRequest, ConferenceCreateResponse};
pub use self::core_data::client::{
    ClientColorDepth, ClientCoreData, ClientCoreOptionalData, ClientEarlyCapabilityFlags, ColorDepth, ConnectionType,
    HighColorDepth, KeyboardType, SecureAccessSequence, SupportedColorDepths, IME_FILE_NAME_SIZE,
};
pub use self::core_data::server::{ServerCoreData, ServerCoreOptionalData, ServerEarlyCapabilityFlags};
pub use self::core_data::{CoreDataError, RdpVersion};
pub use self::message_channel_data::{ClientMessageChannelData, ServerMessageChannelData};
pub use self::monitor_data::{
    ClientMonitorData, Monitor, MonitorFlags, MONITOR_COUNT_SIZE, MONITOR_FLAGS_SIZE, MONITOR_SIZE,
};
pub use self::monitor_extended_data::{ClientMonitorExtendedData, ExtendedMonitorInfo, MonitorOrientation};
pub use self::multi_transport_channel_data::{MultiTransportChannelData, MultiTransportFlags};
pub use self::network_data::{
    ChannelDef, ChannelName, ChannelOptions, ClientNetworkData, NetworkDataError, ServerNetworkData,
};
pub use self::security_data::{
    ClientSecurityData, EncryptionLevel, EncryptionMethod, SecurityDataError, ServerSecurityData,
};

macro_rules! user_header_try {
    ($e:expr) => {
        match $e {
            Ok(user_header) => user_header,
            Err(e) if matches!(e.kind(), PduErrorKind::NotEnoughBytes { .. }) => break,
            Err(e) => return Err(e),
        }
    };
}

const USER_DATA_HEADER_SIZE: usize = 4;

/// 2.2.1.3 Client MCS Connect Initial PDU with GCC Conference Create Request
///
/// [2.2.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/db6713ee-1c0e-4064-a3b3-0fac30b4037b
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientGccBlocks {
    pub core: ClientCoreData,
    pub security: ClientSecurityData,
    /// According to [MSDN](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/c1bea8bd-069c-4437-9769-db5d27935225),
    /// the Client GCC blocks MUST contain Core, Security, Network GCC blocks.
    /// But the FreeRDP does not send the Network GCC block if it does not have channels to join,
    /// and what is surprising - Windows RDP server accepts this GCC block.
    /// Because of this, the Network GCC block is made optional in IronRDP.
    pub network: Option<ClientNetworkData>,
    pub cluster: Option<ClientClusterData>,
    pub monitor: Option<ClientMonitorData>,
    pub message_channel: Option<ClientMessageChannelData>,
    pub multi_transport_channel: Option<MultiTransportChannelData>,
    pub monitor_extended: Option<ClientMonitorExtendedData>,
}

impl ClientGccBlocks {
    const NAME: &'static str = "ClientGccBlocks";

    pub fn channel_names(&self) -> Option<Vec<ChannelDef>> {
        self.network.as_ref().map(|network| network.channels.clone())
    }
}

impl PduEncode for ClientGccBlocks {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        UserDataHeader::encode(dst, ClientGccType::CoreData, &self.core)?;
        UserDataHeader::encode(dst, ClientGccType::SecurityData, &self.security)?;

        if let Some(ref network) = self.network {
            UserDataHeader::encode(dst, ClientGccType::NetworkData, network)?;
        }
        if let Some(ref cluster) = self.cluster {
            UserDataHeader::encode(dst, ClientGccType::ClusterData, cluster)?;
        }
        if let Some(ref monitor) = self.monitor {
            UserDataHeader::encode(dst, ClientGccType::MonitorData, monitor)?;
        }
        if let Some(ref message_channel) = self.message_channel {
            UserDataHeader::encode(dst, ClientGccType::MessageChannelData, message_channel)?;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            UserDataHeader::encode(dst, ClientGccType::MultiTransportChannelData, multi_transport_channel)?;
        }
        if let Some(ref monitor_extended) = self.monitor_extended {
            UserDataHeader::encode(dst, ClientGccType::MonitorExtendedData, monitor_extended)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut size = self.core.size() + self.security.size() + USER_DATA_HEADER_SIZE * 2;

        if let Some(ref network) = self.network {
            size += network.size() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref cluster) = self.cluster {
            size += cluster.size() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref monitor) = self.monitor {
            size += monitor.size() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref message_channel) = self.message_channel {
            size += message_channel.size() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            size += multi_transport_channel.size() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref monitor_extended) = self.monitor_extended {
            size += monitor_extended.size() + USER_DATA_HEADER_SIZE;
        }

        size
    }
}

impl<'de> PduDecode<'de> for ClientGccBlocks {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let mut core = None;
        let mut security = None;
        let mut network = None;
        let mut cluster = None;
        let mut monitor = None;
        let mut message_channel = None;
        let mut multi_transport_channel = None;
        let mut monitor_extended = None;

        loop {
            let (ty, cur) = user_header_try!(UserDataHeader::decode(src));

            match ty {
                ClientGccType::CoreData => core = Some(decode(cur)?),
                ClientGccType::SecurityData => security = Some(decode(cur)?),
                ClientGccType::NetworkData => network = Some(decode(cur)?),
                ClientGccType::ClusterData => cluster = Some(decode(cur)?),
                ClientGccType::MonitorData => monitor = Some(decode(cur)?),
                ClientGccType::MessageChannelData => message_channel = Some(decode(cur)?),
                ClientGccType::MonitorExtendedData => monitor_extended = Some(decode(cur)?),
                ClientGccType::MultiTransportChannelData => multi_transport_channel = Some(decode(cur)?),
            };
        }

        Ok(Self {
            core: core.ok_or_else(|| invalid_message_err!("core", "required GCC core is absent"))?,
            security: security.ok_or_else(|| invalid_message_err!("security", "required GCC security is absent"))?,
            network,
            cluster,
            monitor,
            message_channel,
            multi_transport_channel,
            monitor_extended,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerGccBlocks {
    pub core: ServerCoreData,
    pub network: ServerNetworkData,
    pub security: ServerSecurityData,
    pub message_channel: Option<ServerMessageChannelData>,
    pub multi_transport_channel: Option<MultiTransportChannelData>,
}

impl ServerGccBlocks {
    const NAME: &'static str = "ServerGccBlocks";

    pub fn channel_ids(&self) -> Vec<u16> {
        self.network.channel_ids.clone()
    }
    pub fn global_channel_id(&self) -> u16 {
        self.network.io_channel
    }
}

impl PduEncode for ServerGccBlocks {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        UserDataHeader::encode(dst, ServerGccType::CoreData, &self.core)?;
        UserDataHeader::encode(dst, ServerGccType::NetworkData, &self.network)?;
        UserDataHeader::encode(dst, ServerGccType::SecurityData, &self.security)?;

        if let Some(ref message_channel) = self.message_channel {
            UserDataHeader::encode(dst, ServerGccType::MessageChannelData, message_channel)?;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            UserDataHeader::encode(dst, ServerGccType::MultiTransportChannelData, multi_transport_channel)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut size = self.core.size() + self.network.size() + self.security.size() + USER_DATA_HEADER_SIZE * 3;

        if let Some(ref message_channel) = self.message_channel {
            size += message_channel.size() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            size += multi_transport_channel.size() + USER_DATA_HEADER_SIZE;
        }

        size
    }
}

impl<'de> PduDecode<'de> for ServerGccBlocks {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let mut core = None;
        let mut network = None;
        let mut security = None;
        let mut message_channel = None;
        let mut multi_transport_channel = None;

        loop {
            let (ty, cur) = user_header_try!(UserDataHeader::decode(src));

            match ty {
                ServerGccType::CoreData => core = Some(decode(cur)?),
                ServerGccType::NetworkData => network = Some(decode(cur)?),
                ServerGccType::SecurityData => security = Some(decode(cur)?),
                ServerGccType::MessageChannelData => message_channel = Some(decode(cur)?),
                ServerGccType::MultiTransportChannelData => multi_transport_channel = Some(decode(cur)?),
            };
        }

        Ok(Self {
            core: core.ok_or_else(|| invalid_message_err!("core", "required GCC core is absent"))?,
            network: network.ok_or_else(|| invalid_message_err!("network", "required GCC network is absent"))?,
            security: security.ok_or_else(|| invalid_message_err!("security", "required GCC security is absent"))?,
            message_channel,
            multi_transport_channel,
        })
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, FromPrimitive, ToPrimitive)]
pub enum ClientGccType {
    CoreData = 0xC001,
    SecurityData = 0xC002,
    NetworkData = 0xC003,
    ClusterData = 0xC004,
    MonitorData = 0xC005,
    MessageChannelData = 0xC006,
    MonitorExtendedData = 0xC008,
    MultiTransportChannelData = 0xC00A,
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, FromPrimitive, ToPrimitive)]
pub enum ServerGccType {
    CoreData = 0x0C01,
    SecurityData = 0x0C02,
    NetworkData = 0x0C03,
    MessageChannelData = 0x0C04,
    MultiTransportChannelData = 0x0C08,
}

#[derive(Debug)]
pub struct UserDataHeader;

impl UserDataHeader {
    const FIXED_PART_SIZE: usize = 2 /* blockType */ + 2 /* blockLen */;

    pub fn encode<T, B>(dst: &mut WriteCursor<'_>, block_type: T, block: &B) -> PduResult<()>
    where
        T: ToPrimitive,
        B: PduEncode,
    {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(block_type.to_u16().unwrap());
        dst.write_u16(cast_length!("blockLen", block.size() + USER_DATA_HEADER_SIZE)?);
        block.encode(dst)?;

        Ok(())
    }

    pub fn decode<'de, T>(src: &mut ReadCursor<'de>) -> PduResult<(T, &'de [u8])>
    where
        T: FromPrimitive,
    {
        ensure_fixed_part_size!(in: src);

        let block_type =
            T::from_u16(src.read_u16()).ok_or_else(|| invalid_message_err!("blockType", "invalid GCC type"))?;
        let block_length: usize = cast_length!("blockLen", src.read_u16())?;

        if block_length <= USER_DATA_HEADER_SIZE {
            return Err(invalid_message_err!("blockLen", "invalid UserDataHeader length"));
        }

        let len = block_length - USER_DATA_HEADER_SIZE;
        ensure_size!(in: src, size: len);

        Ok((block_type, src.read_slice(len)))
    }
}

#[derive(Debug, Error)]
pub enum GccError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("core data block error")]
    CoreError(#[from] CoreDataError),
    #[error("security data block error")]
    SecurityError(#[from] SecurityDataError),
    #[error("network data block error")]
    NetworkError(#[from] NetworkDataError),
    #[error("cluster data block error")]
    ClusterError(#[from] ClusterDataError),
    #[error("invalid GCC block type")]
    InvalidGccType,
    #[error("invalid conference create request: {0}")]
    InvalidConferenceCreateRequest(String),
    #[error("invalid Conference create response: {0}")]
    InvalidConferenceCreateResponse(String),
    #[error("a server did not send the required GCC data block: {0:?}")]
    RequiredClientDataBlockIsAbsent(ClientGccType),
    #[error("a client did not send the required GCC data block: {0:?}")]
    RequiredServerDataBlockIsAbsent(ServerGccType),
    #[error("PDU error: {0}")]
    Pdu(PduError),
}

impl From<PduError> for GccError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}
