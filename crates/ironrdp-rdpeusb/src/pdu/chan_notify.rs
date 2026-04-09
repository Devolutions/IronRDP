//! PDU's specific to the [Channel Notification][1] interface.
//!
//! Used by both the client and the server to communicate with the other side. For server-to-client
//! notifications, the default interface ID is `0x00000002`; for client-to-server notifications, the
//! default interface ID is `0x00000003`.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62394399c0

use alloc::borrow::ToOwned as _;

use ironrdp_core::{
    DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, unsupported_value_err,
};

use crate::ensure_payload_size;
use crate::pdu::header::SharedMsgHeader;

/// The `CHANNEL_CREATED` message is sent from both the client and the server to inform the other
/// side of the RDP USB device redirection version supported.
//
/// * [MS-RDPEUSB § 2.2.5.1 Channel Created Message (CHANNEL_CREATED)][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e2859c23-acda-47d4-a2fc-9e7415e4b8d6
///
// NOTE: Implementation of IO_CONTROL message's input_buffer_size and input_buffer fields may
// need modification if a newer version of MS-RDPEUSB is realeased.
#[doc(alias = "CHANNEL_CREATED")]
pub struct ChannelCreated {
    pub header: SharedMsgHeader,
}

impl ChannelCreated {
    pub const PAYLOAD_SIZE: usize = size_of::<u32>() * 3;

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_WHEN_NOT_RSP;

    /// The major version of RDP USB redirection supported.
    #[doc(alias = "MajorVersion")]
    pub const MAJOR_VER: u32 = 1;

    /// The minor version of RDP USB redirection supported.
    #[doc(alias = "MinorVersion")]
    pub const MINOR_VER: u32 = 0;

    /// The capabilities of RDP USB redirection supported.
    #[doc(alias = "Capabilities")]
    pub const CAPS: u32 = 0;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_payload_size!(in: src);
        if src.read_u32() != Self::MAJOR_VER {
            return Err(unsupported_value_err!("MajorVersion", "is not: 1".to_owned()));
        }
        if src.read_u32() != Self::MINOR_VER {
            return Err(unsupported_value_err!("MinorVersion", "is not: 0".to_owned()));
        }
        if src.read_u32() != Self::CAPS {
            return Err(unsupported_value_err!("Capabilities", "is not: 0".to_owned()));
        }
        Ok(Self { header })
    }
}

impl Encode for ChannelCreated {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header.encode(dst)?;

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
