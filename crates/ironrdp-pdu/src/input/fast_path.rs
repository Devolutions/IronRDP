use bit_field::BitField as _;
use bitflags::bitflags;
use ironrdp_core::{
    cast_length, ensure_fixed_part_size, ensure_size, invalid_field_err, other_err, Decode, DecodeResult, Encode,
    EncodeResult, ReadCursor, WriteCursor,
};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};

use crate::fast_path::EncryptionFlags;
use crate::input::{MousePdu, MouseRelPdu, MouseXPdu};
use crate::per;

/// Implements the Fast-Path RDP message header PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathInputHeader {
    pub flags: EncryptionFlags,
    pub data_length: usize,
    pub num_events: u8,
}

impl FastPathInputHeader {
    const NAME: &'static str = "FastPathInputHeader";

    const FIXED_PART_SIZE: usize = 1 /* header */;
}

impl Encode for FastPathInputHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let mut header = 0u8;
        header.set_bits(0..2, 0); // fast-path action
        if self.num_events < 16 {
            header.set_bits(2..7, self.num_events);
        }
        header.set_bits(6..8, self.flags.bits());
        dst.write_u8(header);

        per::write_length(dst, cast_length!("len", self.data_length + self.size())?);
        if self.num_events > 15 {
            dst.write_u8(self.num_events);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let num_events_length = if self.num_events < 16 { 0 } else { 1 };
        Self::FIXED_PART_SIZE
            + per::sizeof_length(self.data_length as u16 + num_events_length as u16 + 1)
            + num_events_length
    }
}

impl<'de> Decode<'de> for FastPathInputHeader {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let header = src.read_u8();
        let flags = EncryptionFlags::from_bits_truncate(header.get_bits(6..8));
        let mut num_events = header.get_bits(2..6);
        let (length, sizeof_length) = per::read_length(src).map_err(|e| other_err!("perLen", source: e))?;

        if !flags.is_empty() {
            return Err(invalid_field_err!("flags", "encryption not supported"));
        }

        let num_events_length = if num_events == 0 {
            ensure_size!(in: src, size: 1);
            num_events = src.read_u8();
            1
        } else {
            0
        };

        let data_length = length as usize - sizeof_length - 1 - num_events_length;

        Ok(FastPathInputHeader {
            flags,
            data_length,
            num_events,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
#[repr(u8)]
pub enum FastpathInputEventType {
    ScanCode = 0x0000,
    Mouse = 0x0001,
    MouseX = 0x0002,
    Sync = 0x0003,
    Unicode = 0x0004,
    MouseRel = 0x0005,
    QoeTimestamp = 0x0006,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastPathInputEvent {
    KeyboardEvent(KeyboardFlags, u8),
    UnicodeKeyboardEvent(KeyboardFlags, u16),
    MouseEvent(MousePdu),
    MouseEventEx(MouseXPdu),
    MouseEventRel(MouseRelPdu),
    QoeEvent(u32),
    SyncEvent(SynchronizeFlags),
}

impl FastPathInputEvent {
    const NAME: &'static str = "FastPathInputEvent";

    const FIXED_PART_SIZE: usize = 1 /* header */;
}

impl Encode for FastPathInputEvent {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let mut header = 0u8;
        let (flags, code) = match self {
            FastPathInputEvent::KeyboardEvent(flags, _) => (flags.bits(), FastpathInputEventType::ScanCode),
            FastPathInputEvent::UnicodeKeyboardEvent(flags, _) => (flags.bits(), FastpathInputEventType::Unicode),
            FastPathInputEvent::MouseEvent(_) => (0, FastpathInputEventType::Mouse),
            FastPathInputEvent::MouseEventEx(_) => (0, FastpathInputEventType::MouseX),
            FastPathInputEvent::MouseEventRel(_) => (0, FastpathInputEventType::MouseRel),
            FastPathInputEvent::QoeEvent(_) => (0, FastpathInputEventType::QoeTimestamp),
            FastPathInputEvent::SyncEvent(flags) => (flags.bits(), FastpathInputEventType::Sync),
        };
        header.set_bits(0..5, flags);
        header.set_bits(5..8, code.to_u8().unwrap());
        dst.write_u8(header);
        match self {
            FastPathInputEvent::KeyboardEvent(_, code) => {
                dst.write_u8(*code);
            }
            FastPathInputEvent::UnicodeKeyboardEvent(_, code) => {
                dst.write_u16(*code);
            }
            FastPathInputEvent::MouseEvent(pdu) => {
                pdu.encode(dst)?;
            }
            FastPathInputEvent::MouseEventEx(pdu) => {
                pdu.encode(dst)?;
            }
            FastPathInputEvent::QoeEvent(stamp) => {
                dst.write_u32(*stamp);
            }
            _ => {}
        };

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            + match self {
                FastPathInputEvent::KeyboardEvent(_, _) => 1,
                FastPathInputEvent::UnicodeKeyboardEvent(_, _) => 2,
                FastPathInputEvent::MouseEvent(pdu) => pdu.size(),
                FastPathInputEvent::MouseEventEx(pdu) => pdu.size(),
                FastPathInputEvent::MouseEventRel(pdu) => pdu.size(),
                FastPathInputEvent::QoeEvent(_) => 4,
                FastPathInputEvent::SyncEvent(_) => 0,
            }
    }
}

impl<'de> Decode<'de> for FastPathInputEvent {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let header = src.read_u8();
        let flags = header.get_bits(0..5);
        let code = header.get_bits(5..8);
        let code: FastpathInputEventType = FastpathInputEventType::from_u8(code)
            .ok_or_else(|| invalid_field_err!("code", "input event code unsupported"))?;
        let event = match code {
            FastpathInputEventType::ScanCode => {
                ensure_size!(in: src, size: 1);
                let code = src.read_u8();
                let flags = KeyboardFlags::from_bits(flags)
                    .ok_or_else(|| invalid_field_err!("flags", "input keyboard flags unsupported"))?;
                FastPathInputEvent::KeyboardEvent(flags, code)
            }
            FastpathInputEventType::Mouse => {
                let mouse_event = MousePdu::decode(src)?;
                FastPathInputEvent::MouseEvent(mouse_event)
            }
            FastpathInputEventType::MouseX => {
                let mouse_event = MouseXPdu::decode(src)?;
                FastPathInputEvent::MouseEventEx(mouse_event)
            }
            FastpathInputEventType::MouseRel => {
                let mouse_event = MouseRelPdu::decode(src)?;
                FastPathInputEvent::MouseEventRel(mouse_event)
            }
            FastpathInputEventType::Sync => {
                let flags = SynchronizeFlags::from_bits(flags)
                    .ok_or_else(|| invalid_field_err!("flags", "input synchronize flags unsupported"))?;
                FastPathInputEvent::SyncEvent(flags)
            }
            FastpathInputEventType::Unicode => {
                ensure_size!(in: src, size: 2);
                let code = src.read_u16();
                let flags = KeyboardFlags::from_bits(flags)
                    .ok_or_else(|| invalid_field_err!("flags", "input keyboard flags unsupported"))?;
                FastPathInputEvent::UnicodeKeyboardEvent(flags, code)
            }
            FastpathInputEventType::QoeTimestamp => {
                ensure_size!(in: src, size: 4);
                let code = src.read_u32();
                FastPathInputEvent::QoeEvent(code)
            }
        };
        Ok(event)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct KeyboardFlags: u8 {
        const RELEASE = 0x01;
        const EXTENDED = 0x02;
        const EXTENDED1 = 0x04;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SynchronizeFlags: u8 {
        const SCROLL_LOCK = 0x01;
        const NUM_LOCK = 0x02;
        const CAPS_LOCK = 0x04;
        const KANA_LOCK = 0x08;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathInput(pub Vec<FastPathInputEvent>);

impl FastPathInput {
    const NAME: &'static str = "FastPathInput";
}

impl Encode for FastPathInput {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        if self.0.is_empty() {
            return Err(other_err!("Empty fast-path input"));
        }

        let data_length = self.0.iter().map(Encode::size).sum::<usize>();
        let header = FastPathInputHeader {
            num_events: self.0.len() as u8,
            flags: EncryptionFlags::empty(),
            data_length,
        };
        header.encode(dst)?;

        for event in self.0.iter() {
            event.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let data_length = self.0.iter().map(Encode::size).sum::<usize>();
        let header = FastPathInputHeader {
            num_events: self.0.len() as u8,
            flags: EncryptionFlags::empty(),
            data_length,
        };
        header.size() + data_length
    }
}

impl<'de> Decode<'de> for FastPathInput {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = FastPathInputHeader::decode(src)?;
        let events = (0..header.num_events)
            .map(|_| FastPathInputEvent::decode(src))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self(events))
    }
}
