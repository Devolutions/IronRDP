//! Messages specific to the [Channel Notification][1] interface.
//!
//! Used by both the client and the server to communicate with the other side. For server-to-client
//! notifications, the default interface ID is `0x00000002`; for client-to-server notifications, the
//! default interface ID is `0x00000003`.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62394399c0

use alloc::format;

use ironrdp_core::{
    DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size,
    unsupported_value_err,
};

use crate::pdu::header::{FunctionId, InterfaceId, Mask, MessageId, SharedMsgHeader, unpack};

/// [\[MS-RDPEUSB\] 2.2.5.1 Channel Created Message (CHANNEL_CREATED)][1] packet.
///
/// Sent from both the client and the server to inform the other side of the RDP USB device
/// redirection version supported.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e2859c23-acda-47d4-a2fc-9e7415e4b8d6
#[doc(alias = "CHANNEL_CREATED")]
#[derive(Debug, PartialEq, Clone)]
pub struct ChannelCreated {
    pub msg_id: MessageId,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    ToServer,
    ToClient,
}

impl ChannelCreated {
    const PAYLOAD_SIZE: usize =
        size_of::<u32>(/* MajorVersion */) + size_of::<u32>(/* MinorVersion */) + size_of::<u32>(/* Capabilities */);

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_REQ;

    /// The major version of RDP USB redirection supported.
    #[doc(alias = "MajorVersion")]
    pub const MAJOR_VER: u32 = 1;

    /// The minor version of RDP USB redirection supported.
    #[doc(alias = "MinorVersion")]
    pub const MINOR_VER: u32 = 0;

    /// The capabilities of RDP USB redirection supported.
    #[doc(alias = "Capabilities")]
    pub const CAPS: u32 = 0;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: if let Direction::ToServer = self.direction {
                InterfaceId::NOTIFY_SERVER
            } else {
                InterfaceId::NOTIFY_CLIENT
            }
            .with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::CHANNEL_CREATED),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let major = src.read_u32();
        if major != Self::MAJOR_VER {
            return Err(unsupported_value_err!("MajorVersion", format!("{major}")));
        }
        let minor = src.read_u32();
        if minor != Self::MINOR_VER {
            return Err(unsupported_value_err!("MinorVersion", format!("{minor}")));
        }
        let capabilities = src.read_u32();
        if capabilities != Self::CAPS {
            return Err(unsupported_value_err!("Capabilities", format!("{capabilities}")));
        }

        Ok(Self {
            msg_id: header.msg_id,
            direction: match unpack(header.iface_id)?.0 {
                InterfaceId::NOTIFY_CLIENT => Direction::ToClient,
                InterfaceId::NOTIFY_SERVER => Direction::ToServer,
                _ => unreachable!("dispatcher must filter interface_id to NOTIFY_CLIENT/NOTIFY_SERVER"),
            },
        })
    }
}

impl Encode for ChannelCreated {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header().encode(dst)?;

        dst.write_u32(Self::MAJOR_VER);
        dst.write_u32(Self::MINOR_VER);
        dst.write_u32(Self::CAPS);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "CHANNEL_CREATED"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}
