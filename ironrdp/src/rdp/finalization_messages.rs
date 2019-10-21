use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{gcc, impl_from_error, PduParsing};

const SYNCHRONIZE_PDU_SIZE: usize = 2 + 2;
const CONTROL_PDU_SIZE: usize = 2 + 2 + 4;
const FONT_PDU_SIZE: usize = 2 * 4;
const SYNCHRONIZE_MESSAGE_TYPE: u16 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct SynchronizePdu {
    pub target_user_id: u16,
}

impl SynchronizePdu {
    pub fn new(target_user_id: u16) -> Self {
        Self { target_user_id }
    }
}

impl PduParsing for SynchronizePdu {
    type Error = FinalizationMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let message_type = stream.read_u16::<LittleEndian>()?;
        if message_type != SYNCHRONIZE_MESSAGE_TYPE {
            return Err(FinalizationMessagesError::InvalidMessageType);
        }

        let target_user_id = stream.read_u16::<LittleEndian>()?;

        Ok(Self { target_user_id })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(SYNCHRONIZE_MESSAGE_TYPE)?;
        stream.write_u16::<LittleEndian>(self.target_user_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SYNCHRONIZE_PDU_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlPdu {
    pub action: ControlAction,
    pub grant_id: u16,
    pub control_id: u32,
}

impl ControlPdu {
    pub fn new(action: ControlAction, grant_id: u16, control_id: u32) -> Self {
        Self {
            action,
            grant_id,
            control_id,
        }
    }
}

impl PduParsing for ControlPdu {
    type Error = FinalizationMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let action = ControlAction::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(FinalizationMessagesError::InvalidControlAction)?;
        let grant_id = stream.read_u16::<LittleEndian>()?;
        let control_id = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            action,
            grant_id,
            control_id,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.action.to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(self.grant_id)?;
        stream.write_u32::<LittleEndian>(self.control_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CONTROL_PDU_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontPdu {
    pub number: u16,
    pub total_number: u16,
    pub flags: SequenceFlags,
    pub entry_size: u16,
}

impl FontPdu {
    pub fn new(number: u16, total_number: u16, flags: SequenceFlags, entry_size: u16) -> Self {
        Self {
            number,
            total_number,
            flags,
            entry_size,
        }
    }
}

impl PduParsing for FontPdu {
    type Error = FinalizationMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let number = stream.read_u16::<LittleEndian>()?;
        let total_number = stream.read_u16::<LittleEndian>()?;
        let flags = SequenceFlags::from_bits(stream.read_u16::<LittleEndian>()?)
            .ok_or(FinalizationMessagesError::InvalidListFlags)?;
        let entry_size = stream.read_u16::<LittleEndian>()?;

        Ok(Self {
            number,
            total_number,
            flags,
            entry_size,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.number)?;
        stream.write_u16::<LittleEndian>(self.total_number)?;
        stream.write_u16::<LittleEndian>(self.flags.bits())?;
        stream.write_u16::<LittleEndian>(self.entry_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        FONT_PDU_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorLayoutPdu {
    pub monitors: Vec<gcc::Monitor>,
}

impl MonitorLayoutPdu {
    pub fn new(monitors: Vec<gcc::Monitor>) -> Self {
        Self { monitors }
    }
}

impl PduParsing for MonitorLayoutPdu {
    type Error = FinalizationMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let monitor_count = stream.read_u32::<LittleEndian>()?;

        let mut monitors = Vec::with_capacity(monitor_count as usize);
        for _ in 0..monitor_count {
            monitors.push(gcc::Monitor::from_buffer(&mut stream)?);
        }

        Ok(Self { monitors })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.monitors.len() as u32)?;

        for monitor in self.monitors.iter() {
            monitor.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        gcc::MONITOR_COUNT_SIZE + self.monitors.len() * gcc::MONITOR_SIZE
    }
}

#[repr(u16)]
#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum ControlAction {
    RequestControl = 1,
    GrantedControl = 2,
    Detach = 3,
    Cooperate = 4,
}

bitflags! {
    pub struct SequenceFlags: u16 {
        const FIRST = 1;
        const LAST = 2;
    }
}

#[derive(Debug, Fail)]
pub enum FinalizationMessagesError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Monitor Data error: {}", _0)]
    MonitorDataError(#[fail(cause)] gcc::MonitorDataError),
    #[fail(display = "Invalid message type field in Synchronize PDU")]
    InvalidMessageType,
    #[fail(display = "Invalid control action field in Control PDU")]
    InvalidControlAction,
    #[fail(display = "Invalid grant id field in Control PDU")]
    InvalidGrantId,
    #[fail(display = "Invalid control id field in Control PDU")]
    InvalidControlId,
    #[fail(display = "Invalid list flags field in Font List PDU")]
    InvalidListFlags,
}

impl_from_error!(
    io::Error,
    FinalizationMessagesError,
    FinalizationMessagesError::IOError
);
impl_from_error!(
    gcc::MonitorDataError,
    FinalizationMessagesError,
    FinalizationMessagesError::MonitorDataError
);
