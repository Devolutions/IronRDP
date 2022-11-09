use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{impl_from_error, PduParsing};

#[cfg(test)]
pub mod test;

pub mod conference_create;
pub(crate) mod monitor_data;

mod cluster_data;
mod core_data;
mod message_channel_data;
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
    ClientMonitorData, Monitor, MonitorDataError, MonitorFlags, MONITOR_COUNT_SIZE, MONITOR_FLAGS_SIZE, MONITOR_SIZE,
};
pub use self::monitor_extended_data::{
    ClientMonitorExtendedData, ExtendedMonitorInfo, MonitorExtendedDataError, MonitorOrientation,
};
pub use self::multi_transport_channel_data::{
    MultiTransportChannelData, MultiTransportChannelDataError, MultiTransportFlags,
};
pub use self::network_data::{Channel, ChannelOptions, ClientNetworkData, NetworkDataError, ServerNetworkData};
pub use self::security_data::{
    ClientSecurityData, EncryptionLevel, EncryptionMethod, SecurityDataError, ServerSecurityData,
};

macro_rules! user_header_try {
    ($e:expr) => {
        match $e {
            Ok(user_header) => user_header,
            Err(GccError::IOError(ref e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
    };
}

const GCC_TYPE_SIZE: usize = 2;
const USER_DATA_HEADER_SIZE: usize = 4;

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
    pub fn channel_names(&self) -> Option<Vec<network_data::Channel>> {
        self.network.as_ref().map(|network| network.channels.clone())
    }
}

impl PduParsing for ClientGccBlocks {
    type Error = GccError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mut core = None;
        let mut security = None;
        let mut network = None;
        let mut cluster = None;
        let mut monitor = None;
        let mut message_channel = None;
        let mut multi_transport_channel = None;
        let mut monitor_extended = None;

        loop {
            let user_header = user_header_try!(UserDataHeader::<ClientGccType>::from_buffer(&mut buffer));

            match user_header.block_type {
                ClientGccType::CoreData => core = Some(ClientCoreData::from_buffer(user_header.block_data.as_slice())?),
                ClientGccType::SecurityData => {
                    security = Some(ClientSecurityData::from_buffer(user_header.block_data.as_slice())?)
                }
                ClientGccType::NetworkData => {
                    network = Some(ClientNetworkData::from_buffer(user_header.block_data.as_slice())?)
                }
                ClientGccType::ClusterData => {
                    cluster = Some(ClientClusterData::from_buffer(user_header.block_data.as_slice())?)
                }
                ClientGccType::MonitorData => {
                    monitor = Some(ClientMonitorData::from_buffer(user_header.block_data.as_slice())?)
                }
                ClientGccType::MessageChannelData => {
                    message_channel = Some(ClientMessageChannelData::from_buffer(
                        user_header.block_data.as_slice(),
                    )?)
                }
                ClientGccType::MonitorExtendedData => {
                    monitor_extended = Some(ClientMonitorExtendedData::from_buffer(
                        user_header.block_data.as_slice(),
                    )?)
                }
                ClientGccType::MultiTransportChannelData => {
                    multi_transport_channel = Some(MultiTransportChannelData::from_buffer(
                        user_header.block_data.as_slice(),
                    )?)
                }
            };
        }

        Ok(Self {
            core: core.ok_or(GccError::RequiredClientDataBlockIsAbsent(ClientGccType::CoreData))?,
            security: security.ok_or(GccError::RequiredClientDataBlockIsAbsent(ClientGccType::SecurityData))?,
            network,
            cluster,
            monitor,
            message_channel,
            multi_transport_channel,
            monitor_extended,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        UserDataHeader::from_gcc_block(ClientGccType::CoreData, &self.core)?.to_buffer(&mut buffer)?;
        UserDataHeader::from_gcc_block(ClientGccType::SecurityData, &self.security)?.to_buffer(&mut buffer)?;

        if let Some(ref network) = self.network {
            UserDataHeader::from_gcc_block(ClientGccType::NetworkData, network)?.to_buffer(&mut buffer)?;
        }
        if let Some(ref cluster) = self.cluster {
            UserDataHeader::from_gcc_block(ClientGccType::ClusterData, cluster)?.to_buffer(&mut buffer)?;
        }
        if let Some(ref monitor) = self.monitor {
            UserDataHeader::from_gcc_block(ClientGccType::MonitorData, monitor)?.to_buffer(&mut buffer)?;
        }
        if let Some(ref message_channel) = self.message_channel {
            UserDataHeader::from_gcc_block(ClientGccType::MessageChannelData, message_channel)?
                .to_buffer(&mut buffer)?;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            UserDataHeader::from_gcc_block(ClientGccType::MultiTransportChannelData, multi_transport_channel)?
                .to_buffer(&mut buffer)?;
        }
        if let Some(ref monitor_extended) = self.monitor_extended {
            UserDataHeader::from_gcc_block(ClientGccType::MonitorExtendedData, monitor_extended)?
                .to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let mut size = self.core.buffer_length() + self.security.buffer_length() + USER_DATA_HEADER_SIZE * 2;

        if let Some(ref network) = self.network {
            size += network.buffer_length() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref cluster) = self.cluster {
            size += cluster.buffer_length() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref monitor) = self.monitor {
            size += monitor.buffer_length() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref message_channel) = self.message_channel {
            size += message_channel.buffer_length() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            size += multi_transport_channel.buffer_length() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref monitor_extended) = self.monitor_extended {
            size += monitor_extended.buffer_length() + USER_DATA_HEADER_SIZE;
        }

        size
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
    pub fn channel_ids(&self) -> Vec<u16> {
        self.network.channel_ids.clone()
    }
    pub fn global_channel_id(&self) -> u16 {
        self.network.io_channel
    }
}

impl PduParsing for ServerGccBlocks {
    type Error = GccError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mut core = None;
        let mut network = None;
        let mut security = None;
        let mut message_channel = None;
        let mut multi_transport_channel = None;

        loop {
            let user_header = user_header_try!(UserDataHeader::<ServerGccType>::from_buffer(&mut buffer));

            match user_header.block_type {
                ServerGccType::CoreData => core = Some(ServerCoreData::from_buffer(user_header.block_data.as_slice())?),
                ServerGccType::NetworkData => {
                    network = Some(ServerNetworkData::from_buffer(user_header.block_data.as_slice())?)
                }
                ServerGccType::SecurityData => {
                    security = Some(ServerSecurityData::from_buffer(user_header.block_data.as_slice())?)
                }
                ServerGccType::MessageChannelData => {
                    message_channel = Some(ServerMessageChannelData::from_buffer(
                        user_header.block_data.as_slice(),
                    )?)
                }
                ServerGccType::MultiTransportChannelData => {
                    multi_transport_channel = Some(MultiTransportChannelData::from_buffer(
                        user_header.block_data.as_slice(),
                    )?)
                }
            };
        }

        Ok(Self {
            core: core.ok_or(GccError::RequiredServerDataBlockIsAbsent(ServerGccType::CoreData))?,
            network: network.ok_or(GccError::RequiredServerDataBlockIsAbsent(ServerGccType::NetworkData))?,
            security: security.ok_or(GccError::RequiredServerDataBlockIsAbsent(ServerGccType::SecurityData))?,
            message_channel,
            multi_transport_channel,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        UserDataHeader::from_gcc_block(ServerGccType::CoreData, &self.core)?.to_buffer(&mut buffer)?;
        UserDataHeader::from_gcc_block(ServerGccType::NetworkData, &self.network)?.to_buffer(&mut buffer)?;
        UserDataHeader::from_gcc_block(ServerGccType::SecurityData, &self.security)?.to_buffer(&mut buffer)?;

        if let Some(ref message_channel) = self.message_channel {
            UserDataHeader::from_gcc_block(ServerGccType::MessageChannelData, message_channel)?
                .to_buffer(&mut buffer)?;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            UserDataHeader::from_gcc_block(ServerGccType::MultiTransportChannelData, multi_transport_channel)?
                .to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let mut size = self.core.buffer_length()
            + self.network.buffer_length()
            + self.security.buffer_length()
            + USER_DATA_HEADER_SIZE * 3;

        if let Some(ref message_channel) = self.message_channel {
            size += message_channel.buffer_length() + USER_DATA_HEADER_SIZE;
        }
        if let Some(ref multi_transport_channel) = self.multi_transport_channel {
            size += multi_transport_channel.buffer_length() + USER_DATA_HEADER_SIZE;
        }

        size
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
pub struct UserDataHeader<T: FromPrimitive + ToPrimitive> {
    block_type: T,
    block_data: Vec<u8>,
}

impl<T: FromPrimitive + ToPrimitive> UserDataHeader<T> {
    fn from_gcc_block<B: PduParsing>(block_type: T, gcc_block: &B) -> Result<Self, GccError>
    where
        GccError: std::convert::From<<B as PduParsing>::Error>,
    {
        let mut block_data = Vec::with_capacity(gcc_block.buffer_length());
        gcc_block.to_buffer(&mut block_data)?;

        Ok(Self { block_type, block_data })
    }

    fn block_length(&self) -> usize {
        self.block_data.len() + USER_DATA_HEADER_SIZE
    }
}

impl<T: FromPrimitive + ToPrimitive> PduParsing for UserDataHeader<T> {
    type Error = GccError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let block_type = T::from_u16(buffer.read_u16::<LittleEndian>()?).ok_or(GccError::InvalidGccType)?;
        let block_length = buffer.read_u16::<LittleEndian>()?;

        if block_length <= USER_DATA_HEADER_SIZE as u16 {
            return Err(GccError::IOError(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid UserDataHeader length",
            )));
        }

        let mut block_data = vec![0; block_length as usize - USER_DATA_HEADER_SIZE];
        buffer.read_exact(&mut block_data)?;

        Ok(Self { block_type, block_data })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.block_type.to_u16().unwrap())?;
        buffer.write_u16::<LittleEndian>(self.block_length() as u16)?;
        buffer.write_all(self.block_data.as_ref())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        GCC_TYPE_SIZE + self.block_data.len()
    }
}

#[derive(Debug, Fail)]
pub enum GccError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Core data block error: {}", _0)]
    CoreError(#[fail(cause)] CoreDataError),
    #[fail(display = "Security data block error: {}", _0)]
    SecurityError(#[fail(cause)] SecurityDataError),
    #[fail(display = "Network data block error: {}", _0)]
    NetworkError(#[fail(cause)] NetworkDataError),
    #[fail(display = "Cluster data block error: {}", _0)]
    ClusterError(#[fail(cause)] ClusterDataError),
    #[fail(display = "Monitor data block error: {}", _0)]
    MonitorError(#[fail(cause)] MonitorDataError),
    #[fail(display = "Multi-transport channel data block error: {}", _0)]
    MultiTransportChannelError(#[fail(cause)] MultiTransportChannelDataError),
    #[fail(display = "Monitor extended data block error: {}", _0)]
    MonitorExtendedError(#[fail(cause)] MonitorExtendedDataError),
    #[fail(display = "Invalid GCC block type")]
    InvalidGccType,
    #[fail(display = "Invalid conference create request: {}", _0)]
    InvalidConferenceCreateRequest(String),
    #[fail(display = "Invalid Conference create response: {}", _0)]
    InvalidConferenceCreateResponse(String),
    #[fail(display = "A server did not send the required GCC data block: {:?}", _0)]
    RequiredClientDataBlockIsAbsent(ClientGccType),
    #[fail(display = "A client did not send the required GCC data block: {:?}", _0)]
    RequiredServerDataBlockIsAbsent(ServerGccType),
}

impl_from_error!(io::Error, GccError, GccError::IOError);
impl_from_error!(CoreDataError, GccError, GccError::CoreError);
impl_from_error!(SecurityDataError, GccError, GccError::SecurityError);
impl_from_error!(NetworkDataError, GccError, GccError::NetworkError);
impl_from_error!(ClusterDataError, GccError, GccError::ClusterError);
impl_from_error!(MonitorDataError, GccError, GccError::MonitorError);
impl_from_error!(
    MultiTransportChannelDataError,
    GccError,
    GccError::MultiTransportChannelError
);
impl_from_error!(MonitorExtendedDataError, GccError, GccError::MonitorExtendedError);
