use std::io;

use crate::{cursor::WriteCursor, PduEncode, PduParsing, PduResult};
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

const RDP_DISPLAY_HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayControlCapsPdu {
    pub max_num_monitors: u32,
    pub max_monitor_area_factora: u32,
    pub max_monitor_area_factorb: u32,
}

impl PduParsing for DisplayControlCapsPdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let max_num_monitors = stream.read_u32::<LittleEndian>()?;
        let max_monitor_area_factora = stream.read_u32::<LittleEndian>()?;
        let max_monitor_area_factorb = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            max_num_monitors,
            max_monitor_area_factora,
            max_monitor_area_factorb,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.max_num_monitors)?;
        stream.write_u32::<LittleEndian>(self.max_monitor_area_factora)?;
        stream.write_u32::<LittleEndian>(self.max_monitor_area_factorb)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        12
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MonitorFlags: u32 {
        const PRIMARY = 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum Orientation {
    Landscape = 0,
    Portrait = 90,
    LandscapeFlipped = 180,
    PortraitFlipped = 270,
}

/// [2.2.2.2.1] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub flags: MonitorFlags,
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
    pub physical_width: u32,
    pub physical_height: u32,
    pub orientation: Orientation,
    pub desktop_scale_factor: u32,
    pub device_scale_factor: u32,
}

const MONITOR_SIZE: usize = 40;
const MONITOR_PDU_HEADER_SIZE: usize = 8;

impl PduParsing for Monitor {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let flags = MonitorFlags::from_bits(stream.read_u32::<LittleEndian>()?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid monitor flags"))?;
        let left = stream.read_u32::<LittleEndian>()?;
        let top = stream.read_u32::<LittleEndian>()?;
        let width = stream.read_u32::<LittleEndian>()?;
        let height = stream.read_u32::<LittleEndian>()?;
        let physical_width = stream.read_u32::<LittleEndian>()?;
        let physical_height = stream.read_u32::<LittleEndian>()?;
        let orientation = Orientation::from_u32(stream.read_u32::<LittleEndian>()?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid monitor orientation"))?;
        let desktop_scale_factor = stream.read_u32::<LittleEndian>()?;
        let device_scale_factor = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            flags,
            left,
            top,
            width,
            height,
            physical_width,
            physical_height,
            orientation,
            desktop_scale_factor,
            device_scale_factor,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.flags.bits())?;
        stream.write_u32::<LittleEndian>(self.left)?;
        stream.write_u32::<LittleEndian>(self.top)?;
        stream.write_u32::<LittleEndian>(self.width)?;
        stream.write_u32::<LittleEndian>(self.height)?;
        stream.write_u32::<LittleEndian>(self.physical_width)?;
        stream.write_u32::<LittleEndian>(self.physical_height)?;
        stream.write_u32::<LittleEndian>(self.orientation.to_u32().unwrap())?;
        stream.write_u32::<LittleEndian>(self.desktop_scale_factor)?;
        stream.write_u32::<LittleEndian>(self.device_scale_factor)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        MONITOR_SIZE
    }
}

/// [2.2.2.2] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/22741217-12a0-4fb8-b5a0-df43905aaf06
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorLayoutPdu {
    pub monitors: Vec<Monitor>,
}

impl PduParsing for MonitorLayoutPdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _size = stream.read_u32::<LittleEndian>()?;
        let num_monitors = stream.read_u32::<LittleEndian>()?;
        let mut monitors = Vec::new();
        for _ in 0..num_monitors {
            monitors.push(Monitor::from_buffer(&mut stream)?);
        }
        Ok(Self { monitors })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(MONITOR_SIZE as u32)?;
        stream.write_u32::<LittleEndian>(self.monitors.len() as u32)?;

        for monitor in &self.monitors {
            monitor.to_buffer(stream.by_ref())?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        MONITOR_PDU_HEADER_SIZE + self.monitors.len() * MONITOR_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPdu {
    DisplayControlCaps(DisplayControlCapsPdu),
}

impl PduParsing for ServerPdu {
    type Error = DisplayPipelineError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let pdu_type =
            ServerPduType::from_u32(stream.read_u32::<LittleEndian>()?).ok_or(DisplayPipelineError::InvalidCmdId)?;
        let pdu_length = stream.read_u32::<LittleEndian>()? as usize;

        let server_pdu = match pdu_type {
            ServerPduType::DisplayControlCaps => {
                ServerPdu::DisplayControlCaps(DisplayControlCapsPdu::from_buffer(&mut stream)?)
            }
        };
        let buffer_length = server_pdu.buffer_length();

        if buffer_length != pdu_length {
            Err(DisplayPipelineError::InvalidPduLength {
                expected: pdu_length,
                actual: buffer_length,
            })
        } else {
            Ok(server_pdu)
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let buffer_length = self.buffer_length();

        stream.write_u32::<LittleEndian>(ServerPduType::from(self).to_u32().unwrap())?;
        stream.write_u32::<LittleEndian>(buffer_length as u32)?;

        match self {
            ServerPdu::DisplayControlCaps(pdu) => pdu.to_buffer(&mut stream).map_err(DisplayPipelineError::from),
        }
    }

    fn buffer_length(&self) -> usize {
        RDP_DISPLAY_HEADER_SIZE
            + match self {
                ServerPdu::DisplayControlCaps(pdu) => pdu.buffer_length(),
            }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ServerPduType {
    DisplayControlCaps = 0x05,
}

impl<'a> From<&'a ServerPdu> for ServerPduType {
    fn from(s: &'a ServerPdu) -> Self {
        match s {
            ServerPdu::DisplayControlCaps(_) => Self::DisplayControlCaps,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPdu {
    DisplayControlMonitorLayout(MonitorLayoutPdu),
}

impl PduParsing for ClientPdu {
    type Error = DisplayPipelineError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let pdu_type =
            ClientPduType::from_u32(stream.read_u32::<LittleEndian>()?).ok_or(DisplayPipelineError::InvalidCmdId)?;
        let pdu_length = stream.read_u32::<LittleEndian>()? as usize;

        let server_pdu = match pdu_type {
            ClientPduType::DisplayControlMonitorLayout => {
                ClientPdu::DisplayControlMonitorLayout(MonitorLayoutPdu::from_buffer(&mut stream)?)
            }
        };
        let buffer_length = server_pdu.buffer_length();

        if buffer_length != pdu_length {
            Err(DisplayPipelineError::InvalidPduLength {
                expected: pdu_length,
                actual: buffer_length,
            })
        } else {
            Ok(server_pdu)
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let buffer_length = self.buffer_length();

        stream.write_u32::<LittleEndian>(ClientPduType::from(self).to_u32().unwrap())?;
        stream.write_u32::<LittleEndian>(buffer_length as u32)?;

        match self {
            ClientPdu::DisplayControlMonitorLayout(pdu) => {
                pdu.to_buffer(&mut stream).map_err(DisplayPipelineError::from)
            }
        }
    }

    fn buffer_length(&self) -> usize {
        RDP_DISPLAY_HEADER_SIZE
            + match self {
                ClientPdu::DisplayControlMonitorLayout(pdu) => pdu.buffer_length(),
            }
    }
}

impl PduEncode for ClientPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.to_buffer(dst).map_err(DisplayPipelineError::from)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        match self {
            ClientPdu::DisplayControlMonitorLayout(_) => "DISPLAYCONTROL_MONITOR_LAYOUT_PDU",
        }
    }

    fn size(&self) -> usize {
        self.buffer_length()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ClientPduType {
    DisplayControlMonitorLayout = 0x02,
}

impl<'a> From<&'a ClientPdu> for ClientPduType {
    fn from(s: &'a ClientPdu) -> Self {
        match s {
            ClientPdu::DisplayControlMonitorLayout(_) => Self::DisplayControlMonitorLayout,
        }
    }
}

#[derive(Debug, Error)]
pub enum DisplayPipelineError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("invalid Header cmd ID")]
    InvalidCmdId,
    #[error("invalid PDU length: expected ({expected}) != actual ({actual})")]
    InvalidPduLength { expected: usize, actual: usize },
}

#[cfg(feature = "std")]
impl ironrdp_error::legacy::ErrorContext for DisplayPipelineError {
    fn context(&self) -> &'static str {
        "display pipeline"
    }
}
