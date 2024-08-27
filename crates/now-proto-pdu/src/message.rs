use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{DecodeResult, EncodeResult, PduDecode, PduEncode};

use crate::{NowExecMessage, NowHeader, NowMessageClass, NowSessionMessage, NowSystemMessage};

/// Wrapper type for messages transferred over the NOW-PROTO communication channel.
///
/// NOW-PROTO: NOW_*_MSG messages
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NowMessage {
    System(NowSystemMessage),
    Session(NowSessionMessage),
    Exec(NowExecMessage),
}

impl NowMessage {
    const NAME: &'static str = "NOW_MSG";
}

impl PduEncode for NowMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::System(msg) => msg.encode(dst),
            Self::Session(msg) => msg.encode(dst),
            Self::Exec(msg) => msg.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::System(msg) => msg.size(),
            Self::Session(msg) => msg.size(),
            Self::Exec(msg) => msg.size(),
        }
    }
}

impl PduDecode<'_> for NowMessage {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match NowMessageClass(header.class.0) {
            NowMessageClass::SYSTEM => Ok(Self::System(NowSystemMessage::decode_from_body(header, src)?)),
            NowMessageClass::SESSION => Ok(Self::Session(NowSessionMessage::decode_from_body(header, src)?)),
            NowMessageClass::EXEC => Ok(Self::Exec(NowExecMessage::decode_from_body(header, src)?)),
            // Handle unknown class; Unknown kind is handled by underlying message type.
            _ => Err(unsupported_message_err!(class: header.class.0, kind: header.kind)),
        }
    }
}
