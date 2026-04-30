//! Messages specific to the [Channel Notification][1] interface.
//!
//! Used by both the client and the server to communicate with the other side. For server-to-client
//! notifications, the default interface ID is `0x00000002`; for client-to-server notifications, the
//! default interface ID is `0x00000003`.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62394399c0

use alloc::format;

use ironrdp_core::{
    DecodeError, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size,
    unsupported_value_err,
};

use crate::pdu::header::{FunctionId, InterfaceId, MessageId, SharedMsgHeader};

#[derive(Debug)]
pub enum ChannelCreatedErr {
    MajorVersion(u32),
    MinorVersion(u32),
    Capabilities(u32),
}

impl core::fmt::Display for ChannelCreatedErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (field, is, expected) = match self {
            Self::MajorVersion(value) => ("MajorVersion", value, 1),
            Self::MinorVersion(value) => ("MinorVersion", value, 0),
            Self::Capabilities(value) => ("Capabilities", value, 0),
        };
        write!(f, "field {field} is: {is:#X}, should be: {expected}")
    }
}

impl core::error::Error for ChannelCreatedErr {}

/// [\[MS-RDPEUSB\] 2.2.5.1 Channel Created Message (CHANNEL_CREATED)][1] packet.
///
/// Sent from both the client and the server to inform the other side of the RDP USB device
/// redirection version supported.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e2859c23-acda-47d4-a2fc-9e7415e4b8d6
#[doc(alias = "CHANNEL_CREATED")]
#[derive(Debug, PartialEq)]
pub struct ChannelCreated {
    pub header: SharedMsgHeader,
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

    pub fn to_client(msg_id: MessageId) -> Self {
        Self {
            header: SharedMsgHeader {
                interface_id: InterfaceId::NOTIFY_CLIENT,
                mask: super::header::Mask::StreamIdProxy,
                msg_id,
                function_id: Some(FunctionId::CHANNEL_CREATED),
            },
        }
    }

    pub fn to_server(msg_id: MessageId) -> Self {
        Self {
            header: SharedMsgHeader {
                interface_id: InterfaceId::NOTIFY_SERVER,
                mask: super::header::Mask::StreamIdProxy,
                msg_id,
                function_id: Some(FunctionId::CHANNEL_CREATED),
            },
        }
    }

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let major = src.read_u32();
        if major != Self::MAJOR_VER {
            let e: DecodeError = unsupported_value_err!("MajorVersion", format!("{major}"));
            return Err(e.with_source(ChannelCreatedErr::MajorVersion(major)));
        }
        let minor = src.read_u32();
        if minor != Self::MINOR_VER {
            let e: DecodeError = unsupported_value_err!("MinorVersion", format!("{minor}"));
            return Err(e.with_source(ChannelCreatedErr::MinorVersion(minor)));
        }
        let capabilities = src.read_u32();
        if capabilities != Self::CAPS {
            let e: DecodeError = unsupported_value_err!("Capabilities", format!("{capabilities}"));
            return Err(e.with_source(ChannelCreatedErr::Capabilities(capabilities)));
        }

        Ok(Self { header })
    }
}

impl Encode for ChannelCreated {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        // ensure_function_id!(
        //     self.header,
        //     "CHANNEL_CREATED",
        //     FunctionId::CHANNEL_CREATED,
        //     "0x100 (CHANNEL_CREATED)"
        // );
        // if self.header.interface_id != InterfaceId::NOTIFY_CLIENT
        //     && self.header.interface_id != InterfaceId::NOTIFY_SERVER
        // {
        //     return Err(invalid_field_err!("CHANNEL_CREATED::Header::InterfaceId", "is not 0x1"));
        // }
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

#[cfg(test)]
mod tests {
    use ironrdp_core::{Decode as _, WriteBuf, encode_buf};

    use super::*;

    #[test]
    fn to_client() {
        let to_client_en = ChannelCreated::to_client(1290);
        let mut buf = WriteBuf::new();
        let en_size = encode_buf(&to_client_en, &mut buf).unwrap();
        assert_eq!(en_size, to_client_en.size());

        let mut buf = ReadCursor::new(buf.filled());
        let header_de = SharedMsgHeader::decode(&mut buf).unwrap();
        let to_client_de = ChannelCreated::decode(&mut buf, header_de).unwrap();
        assert_eq!(to_client_en, to_client_de);
    }

    #[test]
    fn to_server() {
        let to_client_en = ChannelCreated::to_server(1290);
        let mut buf = WriteBuf::new();
        let en_size = encode_buf(&to_client_en, &mut buf).unwrap();
        assert_eq!(en_size, to_client_en.size());

        let mut src = ReadCursor::new(buf.filled());
        let header_de = SharedMsgHeader::decode(&mut src).unwrap();
        let to_client_de = ChannelCreated::decode(&mut src, header_de).unwrap();
        assert_eq!(to_client_en, to_client_de);
    }
}
