use bitflags::bitflags;
use tap::Pipe as _;

use super::RdpVersion;
use crate::cursor::{ReadCursor, WriteCursor};
use crate::nego::SecurityProtocol;
use crate::{PduDecode, PduEncode, PduResult};

const CLIENT_REQUESTED_PROTOCOL_SIZE: usize = 4;
const EARLY_CAPABILITY_FLAGS_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCoreData {
    pub version: RdpVersion,
    pub optional_data: ServerCoreOptionalData,
}

impl ServerCoreData {
    const NAME: &'static str = "ServerCoreData";

    const FIXED_PART_SIZE: usize = 4 /* rdpVersion */;
}

impl PduEncode for ServerCoreData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.version.0);
        self.optional_data.encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.optional_data.size()
    }
}

impl<'de> PduDecode<'de> for ServerCoreData {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version = src.read_u32().pipe(RdpVersion);
        let optional_data = ServerCoreOptionalData::decode(src)?;

        Ok(Self { version, optional_data })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServerCoreOptionalData {
    pub client_requested_protocols: Option<SecurityProtocol>,
    pub early_capability_flags: Option<ServerEarlyCapabilityFlags>,
}

impl ServerCoreOptionalData {
    const NAME: &'static str = "ServerCoreOptionalData";
}

impl PduEncode for ServerCoreOptionalData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        if let Some(value) = self.client_requested_protocols {
            dst.write_u32(value.bits());
        };

        if let Some(value) = self.early_capability_flags {
            dst.write_u32(value.bits());
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut size = 0;

        if self.client_requested_protocols.is_some() {
            size += CLIENT_REQUESTED_PROTOCOL_SIZE;
        }
        if self.early_capability_flags.is_some() {
            size += EARLY_CAPABILITY_FLAGS_SIZE;
        }

        size
    }
}

macro_rules! try_or_return {
    ($expr:expr, $ret:expr) => {
        match $expr {
            Ok(v) => v,
            Err(_) => return Ok($ret),
        }
    };
}

impl<'de> PduDecode<'de> for ServerCoreOptionalData {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let mut optional_data = Self::default();

        optional_data.client_requested_protocols = Some(
            SecurityProtocol::from_bits(try_or_return!(src.try_read_u32("clientReqProtocols"), optional_data))
                .ok_or_else(|| invalid_message_err!("clientReqProtocols", "invalid server security protocol"))?,
        );

        optional_data.early_capability_flags = Some(
            ServerEarlyCapabilityFlags::from_bits(try_or_return!(src.try_read_u32("earlyCapFlags"), optional_data))
                .ok_or_else(|| invalid_message_err!("earlyCapFlags", "invalid early capability flags"))?,
        );

        Ok(optional_data)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ServerEarlyCapabilityFlags: u32 {
        const EDGE_ACTIONS_SUPPORTED_V1 = 0x0000_0001;
        const DYNAMIC_DST_SUPPORTED = 0x0000_0002;
        const EDGE_ACTIONS_SUPPORTED_V2 = 0x0000_0004;
        const SKIP_CHANNELJOIN_SUPPORTED = 0x0000_0008;
        // The source may set any bits
        const _ = !0;
    }
}
