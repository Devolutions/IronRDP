use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduResult};

pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

const RDP_DISPLAY_HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayControlCapsPdu {
    pub max_num_monitors: u32,
    pub max_monitor_area_factora: u32,
    pub max_monitor_area_factorb: u32,
}

impl DisplayControlCapsPdu {
    const NAME: &'static str = "DisplayControlCapsPdu";

    const FIXED_PART_SIZE: usize = 4 /* MaxNumMonitors */ + 4 /* MaxFactorA */ + 4 /* MaxFactorB */;
}

impl PduEncode for DisplayControlCapsPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.max_num_monitors);
        dst.write_u32(self.max_monitor_area_factora);
        dst.write_u32(self.max_monitor_area_factorb);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for DisplayControlCapsPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let max_num_monitors = src.read_u32();
        let max_monitor_area_factora = src.read_u32();
        let max_monitor_area_factorb = src.read_u32();

        Ok(Self {
            max_num_monitors,
            max_monitor_area_factora,
            max_monitor_area_factorb,
        })
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
/// Deprecated in favor of the struct by the same name in crates/ironrdp-dvc.
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

impl Monitor {
    const NAME: &'static str = "DisplayMonitor";

    const FIXED_PART_SIZE: usize = MONITOR_SIZE;
}

impl PduEncode for Monitor {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.flags.bits());
        dst.write_u32(self.left);
        dst.write_u32(self.top);
        dst.write_u32(self.width);
        dst.write_u32(self.height);
        dst.write_u32(self.physical_width);
        dst.write_u32(self.physical_height);
        dst.write_u32(self.orientation.to_u32().unwrap());
        dst.write_u32(self.desktop_scale_factor);
        dst.write_u32(self.device_scale_factor);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Monitor {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = MonitorFlags::from_bits_retain(src.read_u32());
        let left = src.read_u32();
        let top = src.read_u32();
        let width = src.read_u32();
        let height = src.read_u32();
        let physical_width = src.read_u32();
        let physical_height = src.read_u32();
        let orientation = Orientation::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("orientation", "invalid value"))?;
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
}

/// [2.2.2.2] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// Deprecated in favor of the struct by the same name in crates/ironrdp-dvc.
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/22741217-12a0-4fb8-b5a0-df43905aaf06
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorLayoutPdu {
    pub monitors: Vec<Monitor>,
}

impl MonitorLayoutPdu {
    const NAME: &'static str = "MonitorLayoutPdu";

    const FIXED_PART_SIZE: usize = MONITOR_PDU_HEADER_SIZE;
}

impl PduEncode for MonitorLayoutPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(cast_length!("size", MONITOR_SIZE)?);
        dst.write_u32(cast_length!("len", self.monitors.len())?);

        for monitor in &self.monitors {
            monitor.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        MONITOR_PDU_HEADER_SIZE + self.monitors.len() * MONITOR_SIZE
    }
}

impl<'de> PduDecode<'de> for MonitorLayoutPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let _size = src.read_u32();
        let num_monitors = src.read_u32();
        let mut monitors = Vec::new();
        for _ in 0..num_monitors {
            monitors.push(Monitor::decode(src)?);
        }
        Ok(Self { monitors })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPdu {
    DisplayControlCaps(DisplayControlCapsPdu),
}

impl ServerPdu {
    const NAME: &'static str = "DisplayServerPdu";

    const FIXED_PART_SIZE: usize = RDP_DISPLAY_HEADER_SIZE;
}

impl PduEncode for ServerPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let size = self.size();

        ensure_size!(in: dst, size: size);

        dst.write_u32(ServerPduType::from(self).to_u32().unwrap());
        dst.write_u32(cast_length!("len", size)?);

        match self {
            ServerPdu::DisplayControlCaps(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_DISPLAY_HEADER_SIZE
            + match self {
                ServerPdu::DisplayControlCaps(pdu) => pdu.size(),
            }
    }
}

impl<'de> PduDecode<'de> for ServerPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pdu_type = ServerPduType::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("pduType", "invalid PDU type"))?;
        let pdu_length = src.read_u32() as usize;

        let server_pdu = match pdu_type {
            ServerPduType::DisplayControlCaps => ServerPdu::DisplayControlCaps(DisplayControlCapsPdu::decode(src)?),
        };
        let actual_size = server_pdu.size();

        if actual_size != pdu_length {
            Err(not_enough_bytes_err!(actual_size, pdu_length))
        } else {
            Ok(server_pdu)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ServerPduType {
    DisplayControlCaps = 0x05,
}

impl From<&ServerPdu> for ServerPduType {
    fn from(s: &ServerPdu) -> Self {
        match s {
            ServerPdu::DisplayControlCaps(_) => Self::DisplayControlCaps,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPdu {
    DisplayControlMonitorLayout(MonitorLayoutPdu),
}

impl ClientPdu {
    const NAME: &'static str = "DisplayClientPdu";

    const FIXED_PART_SIZE: usize = RDP_DISPLAY_HEADER_SIZE;
}

impl PduEncode for ClientPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let size = self.size();

        ensure_size!(in: dst, size: size);

        dst.write_u32(ClientPduType::from(self).to_u32().unwrap());
        dst.write_u32(cast_length!("len", size)?);

        match self {
            ClientPdu::DisplayControlMonitorLayout(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_DISPLAY_HEADER_SIZE
            + match self {
                ClientPdu::DisplayControlMonitorLayout(pdu) => pdu.size(),
            }
    }
}

impl<'de> PduDecode<'de> for ClientPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pdu_type = ClientPduType::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("pduType", "invalid PDU type"))?;
        let pdu_length = src.read_u32() as usize;

        let client_pdu = match pdu_type {
            ClientPduType::DisplayControlMonitorLayout => {
                ClientPdu::DisplayControlMonitorLayout(MonitorLayoutPdu::decode(src)?)
            }
        };
        let actual_size = client_pdu.size();

        if actual_size != pdu_length {
            Err(not_enough_bytes_err!(actual_size, pdu_length))
        } else {
            Ok(client_pdu)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ClientPduType {
    DisplayControlMonitorLayout = 0x02,
}

impl From<&ClientPdu> for ClientPduType {
    fn from(s: &ClientPdu) -> Self {
        match s {
            ClientPdu::DisplayControlMonitorLayout(_) => Self::DisplayControlMonitorLayout,
        }
    }
}
