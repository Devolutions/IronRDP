use ironrdp_core::{ReadCursor, WriteCursor};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{PduDecode, PduEncode, PduResult};

const MONITOR_COUNT_MAX: usize = 16;
const MONITOR_ATTRIBUTE_SIZE: u32 = 20;

const FLAGS_SIZE: usize = 4;
const MONITOR_ATTRIBUTE_SIZE_FIELD_SIZE: usize = 4;
const MONITOR_COUNT: usize = 4;
const MONITOR_SIZE: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientMonitorExtendedData {
    pub extended_monitors_info: Vec<ExtendedMonitorInfo>,
}

impl ClientMonitorExtendedData {
    const NAME: &'static str = "ClientMonitorExtendedData";

    const FIXED_PART_SIZE: usize = FLAGS_SIZE + MONITOR_ATTRIBUTE_SIZE_FIELD_SIZE + MONITOR_COUNT;
}

impl PduEncode for ClientMonitorExtendedData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(0); // flags
        dst.write_u32(MONITOR_ATTRIBUTE_SIZE); // flags
        dst.write_u32(cast_length!("nMonitors", self.extended_monitors_info.len())?);

        for extended_monitor_info in self.extended_monitors_info.iter().take(MONITOR_COUNT_MAX) {
            extended_monitor_info.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.extended_monitors_info.len() * MONITOR_SIZE
    }
}

impl<'de> PduDecode<'de> for ClientMonitorExtendedData {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let _flags = src.read_u32(); // is unused

        let monitor_attribute_size = src.read_u32();
        if monitor_attribute_size != MONITOR_ATTRIBUTE_SIZE {
            return Err(invalid_message_err!("monitorAttributeSize", "invalid size"));
        }

        let monitor_count = cast_length!("monitorCount", src.read_u32())?;

        if monitor_count > MONITOR_COUNT_MAX {
            return Err(invalid_message_err!("monitorCount", "invalid monitor count"));
        }

        let mut extended_monitors_info = Vec::with_capacity(monitor_count);
        for _ in 0..monitor_count {
            extended_monitors_info.push(ExtendedMonitorInfo::decode(src)?);
        }

        Ok(Self { extended_monitors_info })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedMonitorInfo {
    pub physical_width: u32,
    pub physical_height: u32,
    pub orientation: MonitorOrientation,
    pub desktop_scale_factor: u32,
    pub device_scale_factor: u32,
}

impl ExtendedMonitorInfo {
    const NAME: &'static str = "ExtendedMonitorInfo";

    const FIXED_PART_SIZE: usize = MONITOR_SIZE;
}

impl PduEncode for ExtendedMonitorInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

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

impl<'de> PduDecode<'de> for ExtendedMonitorInfo {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let physical_width = src.read_u32();
        let physical_height = src.read_u32();
        let orientation = MonitorOrientation::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("orientation", "invalid monitor orientation"))?;
        let desktop_scale_factor = src.read_u32();
        let device_scale_factor = src.read_u32();

        Ok(Self {
            physical_width,
            physical_height,
            orientation,
            desktop_scale_factor,
            device_scale_factor,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum MonitorOrientation {
    Landscape = 0,
    Portrait = 90,
    LandscapeFlipped = 180,
    PortraitFlipped = 270,
}
