use bitflags::bitflags;

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode as _, PduEncode, PduResult};

use crate::{system::NowSystemMessageKind, NowHeader, NowMessage, NowMessageClass, NowSystemMessage, NowVarStr};

bitflags! {
    /// NOW_PROTO: NOW_SYSTEM_SHUTDOWN_FLAG_* constants.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NowSystemShutdownFlags: u16 {
        /// Force shutdown
        ///
        /// NOW-PROTO: NOW_SHUTDOWN_FLAG_FORCE
        const FORCE = 0x0001;
        /// Reboot after shutdown
        ///
        /// NOW-PROTO: NOW_SHUTDOWN_FLAG_REBOOT
        const REBOOT = 0x0002;
    }
}

/// The NOW_SYSTEM_SHUTDOWN_MSG structure is used to request a system shutdown.
///
/// NOW_PROTO: NOW_SYSTEM_SHUTDOWN_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowSystemShutdownMsg {
    pub flags: NowSystemShutdownFlags,
    /// This system shutdown timeout, in seconds.
    pub timeout: u32,
    /// Optional shutdown message.
    pub message: NowVarStr,
}

impl NowSystemShutdownMsg {
    const NAME: &'static str = "NOW_SYSTEM_SHUTDOWN_MSG";
    const FIXED_PART_SIZE: usize = 4 /* u32 timeout */;

    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.message.size()
    }

    pub fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timeout = src.read_u32();
        let message = NowVarStr::decode(src)?;

        Ok(Self {
            flags: NowSystemShutdownFlags::from_bits_retain(header.flags),
            timeout,
            message,
        })
    }
}

impl PduEncode for NowSystemShutdownMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::SYSTEM,
            kind: NowSystemMessageKind::SHUTDOWN.0,
            flags: self.flags.bits(),
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.timeout);
        self.message.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl From<NowSystemShutdownMsg> for NowMessage {
    fn from(msg: NowSystemShutdownMsg) -> Self {
        NowMessage::System(NowSystemMessage::Shutdown(msg))
    }
}
