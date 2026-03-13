//! PDU's specific to the [Channel Notification][1] interface.
//!
//! Used by both the client and the server to communicate with the other side. For server-to-client
//! notifications, the default interface ID is `0x00000002`; for client-to-server notifications, the
//! default interface ID is `0x00000003`.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62394399c0

use alloc::borrow::ToOwned as _;

use ironrdp_core::{
    ensure_size, unsupported_value_err, Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor,
};
use ironrdp_pdu::utils::strict_sum;

use crate::pdu::common::SharedMsgHeader;

/// The `CHANNEL_CREATED` message is sent from both the client and the server to inform the other
/// side of the RDP USB device redirection version supported.
///
/// * [MS-RDPEUSB § 2.2.5.1 Channel Created Message (CHANNEL_CREATED)][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e2859c23-acda-47d4-a2fc-9e7415e4b8d6
#[doc(alias = "CHANNEL_CREATED")]
pub struct ChannelCreated {
    header: SharedMsgHeader,
}

impl ChannelCreated {
    /// The major version of RDP USB redirection supported.
    #[doc(alias = "MajorVersion")]
    pub const MAJOR_VER: u32 = 1;

    /// The minor version of RDP USB redirection supported.
    #[doc(alias = "MinorVersion")]
    pub const MINOR_VER: u32 = 0;

    /// The capabilities of RDP USB redirection supported.
    #[doc(alias = "Capabilities")]
    pub const CAPS: u32 = 0;
}

impl Encode for ChannelCreated {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

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
        strict_sum(&[self.header.size()
            + size_of_val(&ChannelCreated::MAJOR_VER)
            + size_of_val(&ChannelCreated::MINOR_VER)
            + size_of_val(&ChannelCreated::CAPS)])
    }
}

impl Decode<'_> for ChannelCreated {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;
        if src.read_u32() != Self::MAJOR_VER {
            return Err(unsupported_value_err!("MajorVersion", "is not: 1".to_owned()));
        }
        if src.read_u32() != Self::MINOR_VER {
            return Err(unsupported_value_err!("MinorVersion", "is not: 0".to_owned()));
        }
        if src.read_u32() != Self::MAJOR_VER {
            return Err(unsupported_value_err!("Capabilities", "is not: 0".to_owned()));
        }
        Ok(Self { header })
    }
}
