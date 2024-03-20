//! Display Control Virtual Channel
//! [[MS-RDPEDISP]]
//!
//! [[MS-RDPEDISP]]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9
use crate::encode_dvc_messages;
use crate::vec;
use crate::Box;
use crate::DvcClientProcessor;
use crate::DvcMessages;
use crate::DvcPduEncode;
use crate::DvcProcessor;
use crate::PduResult;
use crate::SvcMessage;
use crate::Vec;
use bitflags::bitflags;
use ironrdp_pdu::cast_length;
use ironrdp_pdu::cursor::ReadCursor;
use ironrdp_pdu::cursor::WriteCursor;
use ironrdp_pdu::ensure_fixed_part_size;
use ironrdp_pdu::ensure_size;
use ironrdp_pdu::invalid_message_err;
use ironrdp_pdu::other_err;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::PduDecode;
use ironrdp_pdu::PduEncode;
use ironrdp_pdu::PduError;
use ironrdp_svc::impl_as_any;

pub mod client;
pub mod server;

pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

pub enum DisplayControlPdu {
    MonitorLayout(MonitorLayoutPdu),
    Caps(DisplayControlCapsPdu),
}

impl PduEncode for DisplayControlPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            DisplayControlPdu::MonitorLayout(pdu) => pdu.encode(dst),
            DisplayControlPdu::Caps(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            DisplayControlPdu::MonitorLayout(pdu) => pdu.name(),
            DisplayControlPdu::Caps(pdu) => pdu.name(),
        }
    }

    fn size(&self) -> usize {
        match self {
            DisplayControlPdu::MonitorLayout(pdu) => pdu.size(),
            DisplayControlPdu::Caps(pdu) => pdu.size(),
        }
    }
}

impl DvcPduEncode for DisplayControlPdu {}

impl PduDecode<'_> for DisplayControlPdu {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = Header::decode(src)?;
        match header.pdu_type {
            DisplayControlType::MonitorLayout => {
                Ok(DisplayControlPdu::MonitorLayout(MonitorLayoutPdu::decode(header, src)?))
            }
            DisplayControlType::Caps => Ok(DisplayControlPdu::Caps(DisplayControlCapsPdu::decode(header, src)?)),
        }
    }
}

impl From<MonitorLayoutPdu> for DisplayControlPdu {
    fn from(pdu: MonitorLayoutPdu) -> Self {
        DisplayControlPdu::MonitorLayout(pdu)
    }
}

impl From<DisplayControlCapsPdu> for DisplayControlPdu {
    fn from(pdu: DisplayControlCapsPdu) -> Self {
        DisplayControlPdu::Caps(pdu)
    }
}

/// [2.2.1.1] DISPLAYCONTROL_HEADER
///
/// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/3dceb555-2faf-4596-9e74-62be820df8ba
#[derive(Debug)]
pub struct Header {
    pdu_type: DisplayControlType,
    length: usize,
}

impl Header {
    const FIXED_PART_SIZE: usize = 4 /* pdu_type */ + 4 /* length */;

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let pdu_type = DisplayControlType::try_from(src.read_u32())?;
        let length = cast_length!("Length", src.read_u32())?;
        Ok(Self { pdu_type, length })
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u32(cast_length!("Type", self.pdu_type)?);
        dst.write_u32(cast_length!("Length", self.length)?);
        Ok(())
    }

    pub fn size() -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.2.2.2] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/22741217-12a0-4fb8-b5a0-df43905aaf06
#[derive(Debug)]
pub struct MonitorLayoutPdu {
    header: Header,
    pub monitors: Vec<Monitor>,
}

impl MonitorLayoutPdu {
    const FIXED_PART_SIZE: usize = 4 /* MonitorLayoutSize */ + 4 /* NumMonitors */;

    pub fn new(monitors: Vec<Monitor>) -> Self {
        Self {
            header: Header {
                pdu_type: DisplayControlType::MonitorLayout,
                length: (Header::size() + 4 /* MonitorLayoutSize */ + 4 /* NumMonitors */ + (monitors.len() * Monitor::size())),
            },
            monitors,
        }
    }

    fn decode(header: Header, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let monitor_layout_size = src.read_u32();
        let num_monitors = src.read_u32();
        let mut monitors = Vec::new();
        for _ in 0..num_monitors {
            monitors.push(Monitor::decode(src)?);
        }
        Ok(Self { header, monitors })
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        dst.write_u32(cast_length!("MonitorLayoutSize", Monitor::size())?);
        dst.write_u32(cast_length!("NumMonitors", self.monitors.len())?);
        for monitor in &self.monitors {
            monitor.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DISPLAYCONTROL_MONITOR_LAYOUT_PDU"
    }

    fn size(&self) -> usize {
        self.header.length
    }
}

/// [2.2.2.2.1] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
#[derive(Debug)]
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

impl Monitor {
    const FIXED_PART_SIZE: usize = 40;

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let flags = MonitorFlags::from_bits(src.read_u32())
            .ok_or_else(|| invalid_message_err!("MonitorFlags", "Invalid MonitorFlags"))?;
        let left = src.read_u32();
        let top = src.read_u32();
        let width = src.read_u32();
        let height = src.read_u32();
        let physical_width = src.read_u32();
        let physical_height = src.read_u32();
        let orientation = cast_length!("Orientation", src.read_u32())?;
        let desktop_scale_factor = src.read_u32();
        let device_scale_factor = src.read_u32();

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

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.flags.bits());
        dst.write_u32(self.left);
        dst.write_u32(self.top);
        dst.write_u32(self.width);
        dst.write_u32(self.height);
        dst.write_u32(self.physical_width);
        dst.write_u32(self.physical_height);
        dst.write_u32(self.orientation.into());
        dst.write_u32(self.desktop_scale_factor);
        dst.write_u32(self.device_scale_factor);
        Ok(())
    }
    fn size() -> usize {
        Self::FIXED_PART_SIZE
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MonitorFlags: u32 {
        const PRIMARY = 1;
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Landscape = 0,
    Portrait = 90,
    LandscapeFlipped = 180,
    PortraitFlipped = 270,
}

impl From<Orientation> for u32 {
    fn from(value: Orientation) -> u32 {
        match value {
            Orientation::Landscape => 0,
            Orientation::Portrait => 90,
            Orientation::LandscapeFlipped => 180,
            Orientation::PortraitFlipped => 270,
        }
    }
}

impl TryFrom<u32> for Orientation {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Orientation::Landscape,
            90 => Orientation::Portrait,
            180 => Orientation::LandscapeFlipped,
            270 => Orientation::PortraitFlipped,
            _ => return Err(invalid_message_err!("Orientation", "Invalid Orientation")),
        })
    }
}

/// 2.2.2.1 DISPLAYCONTROL_CAPS_PDU
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/8989a211-984e-4ecc-80f3-60694fc4b476
#[derive(Debug)]
pub struct DisplayControlCapsPdu {
    header: Header,
    pub max_num_monitors: u32,
    pub max_monitor_area_factora: u32,
    pub max_monitor_area_factorb: u32,
}

impl DisplayControlCapsPdu {
    const FIXED_PART_SIZE: usize = 4 /* MaxNumMonitors */ + 4 /* MaxMonitorAreaFactorA */ + 4 /* MaxMonitorAreaFactorB */;

    pub fn new(max_num_monitors: u32, max_monitor_area_factora: u32, max_monitor_area_factorb: u32) -> Self {
        Self {
            header: Header {
                pdu_type: DisplayControlType::Caps,
                length: Header::size() + Self::FIXED_PART_SIZE,
            },
            max_num_monitors,
            max_monitor_area_factora,
            max_monitor_area_factorb,
        }
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        dst.write_u32(self.max_num_monitors);
        dst.write_u32(self.max_monitor_area_factora);
        dst.write_u32(self.max_monitor_area_factorb);
        Ok(())
    }

    pub fn decode(header: Header, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let max_num_monitors = src.read_u32();
        let max_monitor_area_factora = src.read_u32();
        let max_monitor_area_factorb = src.read_u32();
        Ok(Self {
            header,
            max_num_monitors,
            max_monitor_area_factora,
            max_monitor_area_factorb,
        })
    }

    pub fn size(&self) -> usize {
        self.header.length
    }

    pub fn name(&self) -> &'static str {
        "DISPLAYCONTROL_CAPS_PDU"
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum DisplayControlType {
    /// DISPLAYCONTROL_PDU_TYPE_CAPS
    Caps = 0x00000005,
    /// DISPLAYCONTROL_PDU_TYPE_MONITOR_LAYOUT
    MonitorLayout = 0x00000002,
}

impl TryFrom<u32> for DisplayControlType {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(match value {
            0x05 => DisplayControlType::Caps,
            0x02 => DisplayControlType::MonitorLayout,
            _ => return Err(invalid_message_err!("DisplayControlType", "Invalid DisplayControlType")),
        })
    }
}

impl From<DisplayControlType> for u32 {
    fn from(value: DisplayControlType) -> u32 {
        match value {
            DisplayControlType::Caps => 0x05,
            DisplayControlType::MonitorLayout => 0x02,
        }
    }
}
