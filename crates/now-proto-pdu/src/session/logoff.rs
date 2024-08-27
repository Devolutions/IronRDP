use ironrdp_core::{Decode, Encode};
use ironrdp_core::{DecodeResult, EncodeResult, ReadCursor, WriteCursor};

use crate::{NowHeader, NowMessage, NowMessageClass, NowSessionMessage, NowSessionMessageKind};

/// The NOW_SESSION_LOGOFF_MSG is used to request a user session logoff.
///
/// NOW_PROTO: NOW_SESSION_LOGOFF_MSG
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct NowSessionLogoffMsg;

impl NowSessionLogoffMsg {
    const NAME: &'static str = "NOW_SESSION_LOGOFF_MSG";
}

impl Encode for NowSessionLogoffMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = NowHeader {
            size: 0,
            class: NowMessageClass::SESSION,
            kind: NowSessionMessageKind::LOGOFF.0,
            flags: 0,
        };

        header.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE
    }
}

impl Decode<'_> for NowSessionLogoffMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowSessionMessageKind(header.kind)) {
            (NowMessageClass::SESSION, NowSessionMessageKind::LOGOFF) => Ok(Self::default()),
            _ => Err(unsupported_message_err!(class: header.class.0, kind: header.kind)),
        }
    }
}

impl From<NowSessionLogoffMsg> for NowMessage {
    fn from(msg: NowSessionLogoffMsg) -> Self {
        NowMessage::Session(NowSessionMessage::Logoff(msg))
    }
}
