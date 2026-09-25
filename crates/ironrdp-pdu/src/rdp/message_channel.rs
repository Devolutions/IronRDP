//! PDUs a client sends on the MCS message channel (MS-RDPBCGR 2.2.1.3.7).
//!
//! Two PDUs travel from client to server on this channel, both framed by a
//! Basic Security Header rather than a Share Control header: the Auto-Detect
//! Response (MS-RDPBCGR 2.2.14.4) and the Initiate Multitransport Response
//! (MS-RDPBCGR 2.2.15.2). The header's flags say which one follows
//! (`SEC_AUTODETECT_RSP` or `SEC_TRANSPORT_RSP`), so decoding dispatches on
//! them and reports a malformed PDU against the type its header names.

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size};

use crate::rdp::autodetect::AutoDetectRspPdu;
use crate::rdp::headers::BasicSecurityHeaderFlags;
use crate::rdp::multitransport::MultitransportResponsePdu;

/// A PDU received from the client on the MCS message channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessageChannelPdu {
    /// An Auto-Detect Response (`SEC_AUTODETECT_RSP`).
    AutoDetectResponse(AutoDetectRspPdu),
    /// An Initiate Multitransport Response (`SEC_TRANSPORT_RSP`).
    MultitransportResponse(MultitransportResponsePdu),
}

impl ClientMessageChannelPdu {
    const NAME: &'static str = "ClientMessageChannelPdu";

    /// Dispatch only needs the leading `flags` field of the Basic Security
    /// Header; the chosen PDU's own decode validates the rest.
    const FLAGS_SIZE: usize = 2 /* flags */;
}

impl Encode for ClientMessageChannelPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::AutoDetectResponse(pdu) => pdu.encode(dst),
            Self::MultitransportResponse(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::AutoDetectResponse(pdu) => pdu.size(),
            Self::MultitransportResponse(pdu) => pdu.size(),
        }
    }
}

impl<'de> Decode<'de> for ClientMessageChannelPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::FLAGS_SIZE);

        let flags = BasicSecurityHeaderFlags::from_bits_truncate(src.peek_u16());

        if flags.contains(BasicSecurityHeaderFlags::TRANSPORT_RSP) {
            MultitransportResponsePdu::decode(src).map(Self::MultitransportResponse)
        } else {
            AutoDetectRspPdu::decode(src).map(Self::AutoDetectResponse)
        }
    }
}
