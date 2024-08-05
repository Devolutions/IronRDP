use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode, PduEncode, PduResult};

use crate::{NowHeader, NowMessage, NowMessageClass, NowSessionMessage, NowSessionMessageKind};

/// The NOW_SESSION_LOCK_MSG is used to request locking the user session.
///
/// NOW_PROTO: NOW_SESSION_LOCK_MSG
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct NowSessionLockMsg;

impl NowSessionLockMsg {
    const NAME: &'static str = "NOW_SESSION_LOCK_MSG";
}

impl PduEncode for NowSessionLockMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: 0,
            class: NowMessageClass::SESSION,
            kind: NowSessionMessageKind::LOCK.0,
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

impl PduDecode<'_> for NowSessionLockMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowSessionMessageKind(header.kind)) {
            (NowMessageClass::SESSION, NowSessionMessageKind::LOCK) => Ok(Self::default()),
            _ => Err(unexpected_message_kind_err!(class: header.class.0, kind: header.kind)),
        }
    }
}

impl From<NowSessionLockMsg> for NowMessage {
    fn from(val: NowSessionLockMsg) -> Self {
        NowMessage::Session(NowSessionMessage::Lock(val))
    }
}
