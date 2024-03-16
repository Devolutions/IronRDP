use bitflags::bitflags;
use ironrdp_dvc::DvcPduEncode;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};

use ironrdp_pdu::{
    cursor::{ReadCursor, WriteCursor},
    ensure_fixed_part_size, invalid_message_err, PduDecode, PduEncode, PduResult,
};

// Advanced Input channel as defined from Freerdp, [here]:
//
// [here]: https://github.com/FreeRDP/FreeRDP/blob/master/include/freerdp/channels/ainput.h

const VERSION_MAJOR: u32 = 1;
const VERSION_MINOR: u32 = 0;

pub const CHANNEL_NAME: &str = "FreeRDP::Advanced::Input";

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MouseEventFlags: u64 {
        const WHEEL = 0x0000_0001;
        const MOVE = 0x0000_0004;
        const DOWN = 0x0000_0008;

        const REL = 0x0000_0010;
        const HAVE_REL = 0x0000_0020;
        const BUTTON1 = 0x0000_1000; /* left */
        const BUTTON2 = 0x0000_2000; /* right */
        const BUTTON3 = 0x0000_4000; /* middle */

        const XBUTTON1 = 0x0000_0100;
        const XBUTTON2 = 0x0000_0200;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPdu {
    major_version: u32,
    minor_version: u32,
}

impl VersionPdu {
    const NAME: &'static str = "AInputVersionPdu";

    const FIXED_PART_SIZE: usize = 4 /* MajorVersion */ + 4 /* MinorVersion */;

    pub fn new() -> Self {
        Self {
            major_version: VERSION_MAJOR,
            minor_version: VERSION_MINOR,
        }
    }
}

impl Default for VersionPdu {
    fn default() -> Self {
        Self::new()
    }
}

impl PduEncode for VersionPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.major_version);
        dst.write_u32(self.minor_version);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for VersionPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let major_version = src.read_u32();
        let minor_version = src.read_u32();

        Ok(Self {
            major_version,
            minor_version,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ServerPduType {
    Version = 0x01,
}

impl<'a> From<&'a ServerPdu> for ServerPduType {
    fn from(s: &'a ServerPdu) -> Self {
        match s {
            ServerPdu::Version(_) => Self::Version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPdu {
    Version(VersionPdu),
}

impl ServerPdu {
    const NAME: &'static str = "AInputServerPdu";

    const FIXED_PART_SIZE: usize = 2 /* PduType */;
}

impl PduEncode for ServerPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(ServerPduType::from(self).to_u16().unwrap());
        match self {
            ServerPdu::Version(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(match self {
                ServerPdu::Version(pdu) => pdu.size(),
            })
            .expect("never overflow")
    }
}

impl DvcPduEncode for ServerPdu {}

impl<'de> PduDecode<'de> for ServerPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pdu_type = ServerPduType::from_u16(src.read_u16())
            .ok_or_else(|| invalid_message_err!("pduType", "invalid pdu type"))?;

        let server_pdu = match pdu_type {
            ServerPduType::Version => ServerPdu::Version(VersionPdu::decode(src)?),
        };

        Ok(server_pdu)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MousePdu {
    pub time: u64,
    pub flags: MouseEventFlags,
    pub x: i32,
    pub y: i32,
}

impl MousePdu {
    const NAME: &'static str = "AInputMousePdu";

    const FIXED_PART_SIZE: usize = 8 /* Time */ + 8 /* Flags */ + 4 /* X */ + 4 /* Y */;
}

impl PduEncode for MousePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u64(self.time);
        dst.write_u64(self.flags.bits());
        dst.write_i32(self.x);
        dst.write_i32(self.y);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for MousePdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let time = src.read_u64();
        let flags = MouseEventFlags::from_bits_retain(src.read_u64());
        let x = src.read_i32();
        let y = src.read_i32();

        Ok(Self { time, flags, x, y })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPdu {
    Mouse(MousePdu),
}

impl ClientPdu {
    const NAME: &'static str = "AInputClientPdu";

    const FIXED_PART_SIZE: usize = 2 /* PduType */;
}

impl PduEncode for ClientPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(ClientPduType::from(self).to_u16().unwrap());
        match self {
            ClientPdu::Mouse(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            .checked_add(match self {
                ClientPdu::Mouse(pdu) => pdu.size(),
            })
            .expect("never overflow")
    }
}

impl<'de> PduDecode<'de> for ClientPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pdu_type = ClientPduType::from_u16(src.read_u16())
            .ok_or_else(|| invalid_message_err!("pduType", "invalid pdu type"))?;

        let client_pdu = match pdu_type {
            ClientPduType::Mouse => ClientPdu::Mouse(MousePdu::decode(src)?),
        };

        Ok(client_pdu)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ClientPduType {
    Mouse = 0x02,
}

impl<'a> From<&'a ClientPdu> for ClientPduType {
    fn from(s: &'a ClientPdu) -> Self {
        match s {
            ClientPdu::Mouse(_) => Self::Mouse,
        }
    }
}
