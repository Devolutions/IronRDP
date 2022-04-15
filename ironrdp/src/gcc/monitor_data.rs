#[cfg(test)]
pub mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;

use crate::{impl_from_error, PduParsing};

pub const MONITOR_COUNT_SIZE: usize = 4;
pub const MONITOR_SIZE: usize = 20;
pub const MONITOR_FLAGS_SIZE: usize = 4;

const MONITOR_COUNT_MAX: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientMonitorData {
    pub monitors: Vec<Monitor>,
}

impl PduParsing for ClientMonitorData {
    type Error = MonitorDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let _flags = buffer.read_u32::<LittleEndian>()?; // is unused
        let monitor_count = buffer.read_u32::<LittleEndian>()?;

        if monitor_count > MONITOR_COUNT_MAX as u32 {
            return Err(MonitorDataError::InvalidMonitorCount);
        }

        let mut monitors = Vec::with_capacity(monitor_count as usize);
        for _ in 0..monitor_count {
            monitors.push(Monitor::from_buffer(&mut buffer)?);
        }

        Ok(Self { monitors })
    }
    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(0)?; // flags
        buffer.write_u32::<LittleEndian>(self.monitors.len() as u32)?;

        for monitor in self.monitors.iter().take(MONITOR_COUNT_MAX) {
            monitor.to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        MONITOR_FLAGS_SIZE + MONITOR_COUNT_SIZE + self.monitors.len() * MONITOR_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub flags: MonitorFlags,
}

impl PduParsing for Monitor {
    type Error = MonitorDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let left = buffer.read_i32::<LittleEndian>()?;
        let top = buffer.read_i32::<LittleEndian>()?;
        let right = buffer.read_i32::<LittleEndian>()?;
        let bottom = buffer.read_i32::<LittleEndian>()?;
        let flags =
            MonitorFlags::from_bits(buffer.read_u32::<LittleEndian>()?).ok_or(MonitorDataError::InvalidMonitorFlags)?;

        Ok(Self {
            left,
            top,
            right,
            bottom,
            flags,
        })
    }
    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_i32::<LittleEndian>(self.left)?;
        buffer.write_i32::<LittleEndian>(self.top)?;
        buffer.write_i32::<LittleEndian>(self.right)?;
        buffer.write_i32::<LittleEndian>(self.bottom)?;
        buffer.write_u32::<LittleEndian>(self.flags.bits())?;

        Ok(())
    }
    fn buffer_length(&self) -> usize {
        MONITOR_SIZE
    }
}

bitflags! {
    pub struct MonitorFlags: u32 {
        const PRIMARY = 1;
    }
}

#[derive(Debug, Fail)]
pub enum MonitorDataError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid monitor count field")]
    InvalidMonitorCount,
    #[fail(display = "Invalid monitor flags field")]
    InvalidMonitorFlags,
}

impl_from_error!(io::Error, MonitorDataError, MonitorDataError::IOError);
