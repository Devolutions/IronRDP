use std::io::{self};

use bit_field::BitField;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::fast_path::EncryptionFlags;
use crate::input::{InputEventError, MousePdu, MouseXPdu};
use crate::{per, PduParsing};

/// Implements the Fast-Path RDP message header PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathInputHeader {
    pub flags: EncryptionFlags,
    pub data_length: usize,
    pub num_events: u8,
}

impl PduParsing for FastPathInputHeader {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let header = stream.read_u8()?;
        let flags = EncryptionFlags::from_bits_truncate(header.get_bits(6..8));
        let mut num_events = header.get_bits(2..6);
        let (length, sizeof_length) = per::read_length(&mut stream)?;

        if !flags.is_empty() {
            return Err(InputEventError::EncryptionNotSupported);
        }

        let num_events_length = if num_events == 0 {
            num_events = stream.read_u8()?;
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

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let mut header = 0u8;
        header.set_bits(0..2, 0); // fast-path action
        if self.num_events < 16 {
            header.set_bits(2..7, self.num_events);
        }
        header.set_bits(6..8, self.flags.bits());
        stream.write_u8(header)?;

        per::write_length(&mut stream, (self.data_length + self.buffer_length()) as u16)?;
        if self.num_events > 15 {
            stream.write_u8(self.num_events)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let num_events_length = if self.num_events < 16 { 0 } else { 1 };
        1 + per::sizeof_length(self.data_length as u16 + num_events_length as u16 + 1) + num_events_length
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
    QoeTimestamp = 0x0006,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastPathInputEvent {
    KeyboardEvent(KeyboardFlags, u8),
    UnicodeKeyboardEvent(KeyboardFlags, u16),
    MouseEvent(MousePdu),
    MouseEventEx(MouseXPdu),
    QoeEvent(u32),
    SyncEvent(SynchronizeFlags),
}

impl PduParsing for FastPathInputEvent {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let header = stream.read_u8()?;
        let flags = header.get_bits(0..5);
        let code = header.get_bits(5..8);
        let code: FastpathInputEventType =
            FastpathInputEventType::from_u8(code).ok_or(InputEventError::EventCodeUnsupported(code))?;
        let event = match code {
            FastpathInputEventType::ScanCode => {
                let code = stream.read_u8()?;
                let flags = KeyboardFlags::from_bits(flags).ok_or(InputEventError::KeyboardFlagsUnsupported(flags))?;
                FastPathInputEvent::KeyboardEvent(flags, code)
            }
            FastpathInputEventType::Mouse => {
                let mouse_event = MousePdu::from_buffer(stream)?;
                FastPathInputEvent::MouseEvent(mouse_event)
            }
            FastpathInputEventType::MouseX => {
                let mouse_event = MouseXPdu::from_buffer(stream)?;
                FastPathInputEvent::MouseEventEx(mouse_event)
            }
            FastpathInputEventType::Sync => {
                let flags =
                    SynchronizeFlags::from_bits(flags).ok_or(InputEventError::SynchronizeFlagsUnsupported(flags))?;
                FastPathInputEvent::SyncEvent(flags)
            }
            FastpathInputEventType::Unicode => {
                let code = stream.read_u16::<LittleEndian>()?;
                let flags = KeyboardFlags::from_bits(flags).ok_or(InputEventError::KeyboardFlagsUnsupported(flags))?;
                FastPathInputEvent::UnicodeKeyboardEvent(flags, code)
            }
            FastpathInputEventType::QoeTimestamp => {
                let code = stream.read_u32::<LittleEndian>()?;
                FastPathInputEvent::QoeEvent(code)
            }
        };
        Ok(event)
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let mut header = 0u8;
        let (flags, code) = match self {
            FastPathInputEvent::KeyboardEvent(flags, _) => (flags.bits(), FastpathInputEventType::ScanCode),
            FastPathInputEvent::UnicodeKeyboardEvent(flags, _) => (flags.bits(), FastpathInputEventType::Unicode),
            FastPathInputEvent::MouseEvent(_) => (0, FastpathInputEventType::Mouse),
            FastPathInputEvent::MouseEventEx(_) => (0, FastpathInputEventType::MouseX),
            FastPathInputEvent::QoeEvent(_) => (0, FastpathInputEventType::QoeTimestamp),
            FastPathInputEvent::SyncEvent(flags) => (flags.bits(), FastpathInputEventType::Sync),
        };
        header.set_bits(0..5, flags);
        header.set_bits(5..8, code.to_u8().unwrap());
        stream.write_u8(header)?;
        match self {
            FastPathInputEvent::KeyboardEvent(_, code) => {
                stream.write_u8(*code)?;
            }
            FastPathInputEvent::UnicodeKeyboardEvent(_, code) => {
                stream.write_u16::<LittleEndian>(*code)?;
            }
            FastPathInputEvent::MouseEvent(pdu) => {
                pdu.to_buffer(stream)?;
            }
            FastPathInputEvent::MouseEventEx(pdu) => {
                pdu.to_buffer(stream)?;
            }
            FastPathInputEvent::QoeEvent(stamp) => {
                stream.write_u32::<LittleEndian>(*stamp)?;
            }
            _ => {}
        };
        Ok(())
    }

    fn buffer_length(&self) -> usize {
        1 + match self {
            FastPathInputEvent::KeyboardEvent(_, _) => 1,
            FastPathInputEvent::UnicodeKeyboardEvent(_, _) => 2,
            FastPathInputEvent::MouseEvent(pdu) => pdu.buffer_length(),
            FastPathInputEvent::MouseEventEx(pdu) => pdu.buffer_length(),
            FastPathInputEvent::QoeEvent(_) => 4,
            FastPathInputEvent::SyncEvent(_) => 0,
        }
    }
}

bitflags! {
    pub struct KeyboardFlags: u8 {
        const FASTPATH_INPUT_KBDFLAGS_RELEASE = 0x01;
        const FASTPATH_INPUT_KBDFLAGS_EXTENDED = 0x02;
        const FASTPATH_INPUT_KBDFLAGS_EXTENDED1 = 0x04;
    }
}

bitflags! {
    pub struct SynchronizeFlags: u8 {
        const FASTPATH_INPUT_SYNC_SCROLL_LOCK = 0x01;
        const FASTPATH_INPUT_SYNC_NUM_LOCK = 0x02;
        const FASTPATH_INPUT_SYNC_CAPS_LOCK = 0x04;
        const FASTPATH_INPUT_SYNC_KANA_LOCK = 0x08;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathInput(pub Vec<FastPathInputEvent>);

impl PduParsing for FastPathInput {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let header = FastPathInputHeader::from_buffer(&mut stream)?;
        let events = (0..header.num_events)
            .map(|_| FastPathInputEvent::from_buffer(&mut stream))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self(events))
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        if self.0.is_empty() {
            return Err(InputEventError::EmptyFastPathInput);
        }

        let data_length = self.0.iter().map(PduParsing::buffer_length).sum::<usize>();
        let header = FastPathInputHeader {
            num_events: self.0.len() as u8,
            flags: EncryptionFlags::empty(),
            data_length,
        };
        header.to_buffer(&mut stream)?;

        for event in self.0.iter() {
            event.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let data_length = self.0.iter().map(PduParsing::buffer_length).sum::<usize>();
        let header = FastPathInputHeader {
            num_events: self.0.len() as u8,
            flags: EncryptionFlags::empty(),
            data_length,
        };
        header.buffer_length() + data_length
    }
}
