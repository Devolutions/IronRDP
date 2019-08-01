#[cfg(test)]
pub mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::PduParsing;

const MONITOR_COUNT_MAX: usize = 16;
const MONITOR_ATTRIBUTE_SIZE: u32 = 20;

const FLAGS_SIZE: usize = 4;
const MONITOR_ATTRIBUTE_SIZE_FIELD_SIZE: usize = 4;
const MONITOR_COUNT: usize = 4;
const MONITOR_SIZE: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientMonitorExtendedData {
    pub extended_monitors_info: Vec<ExtendedMonitorInfo>,
}

impl PduParsing for ClientMonitorExtendedData {
    type Error = MonitorExtendedDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let _flags = buffer.read_u32::<LittleEndian>()?; // is unused

        let monitor_attribute_size = buffer.read_u32::<LittleEndian>()?;
        if monitor_attribute_size != MONITOR_ATTRIBUTE_SIZE {
            return Err(MonitorExtendedDataError::InvalidMonitorAttributeSize);
        }

        let monitor_count = buffer.read_u32::<LittleEndian>()?;

        if monitor_count > MONITOR_COUNT_MAX as u32 {
            return Err(MonitorExtendedDataError::InvalidMonitorCount);
        }

        let mut extended_monitors_info = Vec::with_capacity(monitor_count as usize);
        for _ in 0..monitor_count {
            extended_monitors_info.push(ExtendedMonitorInfo::from_buffer(&mut buffer)?);
        }

        Ok(Self {
            extended_monitors_info,
        })
    }
    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(0)?; // flags
        buffer.write_u32::<LittleEndian>(MONITOR_ATTRIBUTE_SIZE)?; // flags
        buffer.write_u32::<LittleEndian>(self.extended_monitors_info.len() as u32)?;

        for extended_monitor_info in self.extended_monitors_info.iter().take(MONITOR_COUNT_MAX) {
            extended_monitor_info.to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        FLAGS_SIZE
            + MONITOR_ATTRIBUTE_SIZE_FIELD_SIZE
            + MONITOR_COUNT
            + self.extended_monitors_info.len() * MONITOR_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtendedMonitorInfo {
    pub physical_width: u32,
    pub physical_height: u32,
    pub orientation: MonitorOrientation,
    pub desktop_scale_factor: u32,
    pub device_scale_factor: u32,
}

impl PduParsing for ExtendedMonitorInfo {
    type Error = MonitorExtendedDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let physical_width = buffer.read_u32::<LittleEndian>()?;
        let physical_height = buffer.read_u32::<LittleEndian>()?;
        let orientation = MonitorOrientation::from_u32(buffer.read_u32::<LittleEndian>()?)
            .ok_or(MonitorExtendedDataError::InvalidMonitorOrientation)?;
        let desktop_scale_factor = buffer.read_u32::<LittleEndian>()?;
        let device_scale_factor = buffer.read_u32::<LittleEndian>()?;

        Ok(Self {
            physical_width,
            physical_height,
            orientation,
            desktop_scale_factor,
            device_scale_factor,
        })
    }
    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.physical_width)?;
        buffer.write_u32::<LittleEndian>(self.physical_height)?;
        buffer.write_u32::<LittleEndian>(self.orientation.to_u32().unwrap())?;
        buffer.write_u32::<LittleEndian>(self.desktop_scale_factor)?;
        buffer.write_u32::<LittleEndian>(self.device_scale_factor)?;

        Ok(())
    }
    fn buffer_length(&self) -> usize {
        MONITOR_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum MonitorOrientation {
    Landscape = 0,
    Portrait = 90,
    LandscapeFlipped = 180,
    PortraitFlipped = 270,
}

#[derive(Debug, Fail)]
pub enum MonitorExtendedDataError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid monitor attribute size field")]
    InvalidMonitorAttributeSize,
    #[fail(display = "Invalid monitor orientation field")]
    InvalidMonitorOrientation,
    #[fail(display = "Invalid monitor count field")]
    InvalidMonitorCount,
}

impl_from_error!(
    io::Error,
    MonitorExtendedDataError,
    MonitorExtendedDataError::IOError
);
