//! Display Update Virtual Channel Extension PDUs  [MS-RDPEDISP][1] implementation.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{ensure_fixed_part_size, invalid_message_err, PduDecode, PduEncode, PduResult};

const DISPLAYCONTROL_PDU_TYPE_CAPS: u32 = 0x00000005;
const DISPLAYCONTROL_PDU_TYPE_MONITOR_LAYOUT: u32 = 0x00000002;

const DISPLAYCONTROL_MONITOR_PRIMARY: u32 = 0x00000001;

// Set out expectations about supported PDU values. 1024 monitors with 8k*8k pixel area is
// already excessive, (this extension only supports displays up to 8k*8k) therefore we could safely
// use those limits to detect ill-formed PDUs and set out invariants.
const MAX_SUPPORTED_MONITORS: u16 = 1024;
const MAX_MONITOR_AREA_FACTOR: u16 = 1024 * 16;

/// Display Update Virtual Channel message (PDU prefixed with `DISPLAYCONTROL_HEADER`)
///
/// INVARIANTS: size of encoded inner PDU is always less than `u32::MAX - Self::FIXED_PART_SIZE`
///     (See [`DisplayControlCapabilities`] & [`DisplayControlMonitorLayout`] invariants)
#[derive(Debug, Clone)]
pub enum DisplayControlPdu {
    Caps(DisplayControlCapabilities),
    MonitorLayout(DisplayControlMonitorLayout),
}

impl DisplayControlPdu {
    const NAME: &'static str = "DISPLAYCONTROL_HEADER";
    const FIXED_PART_SIZE: usize = 4 /* Type */ + 4 /* Length */;
}

impl PduEncode for DisplayControlPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let (kind, payload_length) = match self {
            DisplayControlPdu::Caps(caps) => (DISPLAYCONTROL_PDU_TYPE_CAPS, caps.size()),
            DisplayControlPdu::MonitorLayout(layout) => (DISPLAYCONTROL_PDU_TYPE_MONITOR_LAYOUT, layout.size()),
        };

        // This will never overflow as per invariants.
        let pdu_size = payload_length.checked_add(Self::FIXED_PART_SIZE).unwrap();

        // Write `DISPLAYCONTROL_HEADER` fields.
        dst.write_u32(kind);
        dst.write_u32(pdu_size.try_into().unwrap());

        match self {
            DisplayControlPdu::Caps(caps) => caps.encode(dst),
            DisplayControlPdu::MonitorLayout(layout) => layout.encode(dst),
        }?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for DisplayControlPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        // Read `DISPLAYCONTROL_HEADER` fields.
        let kind = src.read_u32();
        let pdu_length = src.read_u32();

        let _payload_length = pdu_length
            .checked_sub(Self::FIXED_PART_SIZE.try_into().unwrap())
            .ok_or_else(|| invalid_message_err!("Length", "Display control PDU length is too small"))?;

        match kind {
            DISPLAYCONTROL_PDU_TYPE_CAPS => {
                let caps = DisplayControlCapabilities::decode(src)?;
                Ok(DisplayControlPdu::Caps(caps))
            }
            DISPLAYCONTROL_PDU_TYPE_MONITOR_LAYOUT => {
                let layout = DisplayControlMonitorLayout::decode(src)?;
                Ok(DisplayControlPdu::MonitorLayout(layout))
            }
            _ => Err(invalid_message_err!("Type", "Unknown display control PDU type")),
        }
    }
}

/// Display control channel capabilities PDU.
///
/// INVARIANTS:
///     0 <= max_num_monitors <= MAX_SUPPORTED_MONITORS
///     0 <= max_monitor_area_factor_a <= MAX_MONITOR_AREA_FACTOR
///     0 <= max_monitor_area_factor_b <= MAX_MONITOR_AREA_FACTOR
#[derive(Debug, Clone)]
pub struct DisplayControlCapabilities {
    max_num_monitors: u32,
    max_monitor_area_factor_a: u32,
    max_monitor_area_factor_b: u32,
    max_monitor_area: u64,
}

impl DisplayControlCapabilities {
    const NAME: &'static str = "DISPLAYCONTROL_CAPS_PDU";
    const FIXED_PART_SIZE: usize = 4 /* MaxNumMonitors */
        + 4 /* MaxMonitorAreaFactorA */
        + 4 /* MaxMonitorAreaFactorB */;

    pub fn new(
        max_num_monitors: u32,
        max_monitor_area_factor_a: u32,
        max_monitor_area_factor_b: u32,
    ) -> PduResult<Self> {
        let max_monitor_area =
            calculate_monitor_area(max_num_monitors, max_monitor_area_factor_a, max_monitor_area_factor_b)?;

        Ok(Self {
            max_num_monitors,
            max_monitor_area_factor_a,
            max_monitor_area_factor_b,
            max_monitor_area,
        })
    }

    pub fn max_monitor_area(&self) -> u64 {
        self.max_monitor_area
    }
}

impl PduEncode for DisplayControlCapabilities {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.max_num_monitors);
        dst.write_u32(self.max_monitor_area_factor_a);
        dst.write_u32(self.max_monitor_area_factor_b);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for DisplayControlCapabilities {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let max_num_monitors = src.read_u32();
        let max_monitor_area_factor_a = src.read_u32();
        let max_monitor_area_factor_b = src.read_u32();

        let max_monitor_area =
            calculate_monitor_area(max_num_monitors, max_monitor_area_factor_a, max_monitor_area_factor_b)?;

        Ok(Self {
            max_num_monitors,
            max_monitor_area_factor_a,
            max_monitor_area_factor_b,
            max_monitor_area,
        })
    }
}

/// Sent from client to server to notify about new monitor layout (e.g screen resize).
///
/// INVARIANTS:
///     0 <= monitors.length() <= MAX_SUPPORTED_MONITORS
#[derive(Debug, Clone)]
pub struct DisplayControlMonitorLayout {
    monitors: Vec<MonitorLayoutEntry>,
}

impl DisplayControlMonitorLayout {
    const NAME: &'static str = "DISPLAYCONTROL_MONITOR_LAYOUT_PDU";
    const FIXED_PART_SIZE: usize = 4 /* MonitorLayoutSize */ + 4 /* NumMonitors */;

    pub fn new(monitors: &[MonitorLayoutEntry]) -> PduResult<Self> {
        if monitors.len() > MAX_SUPPORTED_MONITORS.into() {
            return Err(invalid_message_err!("NumMonitors", "Too many monitors",));
        }

        let primary_monitors_count = monitors.iter().filter(|monitor| monitor.is_primary()).count();

        if primary_monitors_count != 1 {
            return Err(invalid_message_err!(
                "PrimaryMonitor",
                "There must be exactly one primary monitor"
            ));
        }

        Ok(Self {
            monitors: monitors.to_vec(),
        })
    }

    pub fn monitors(&self) -> &[MonitorLayoutEntry] {
        &self.monitors
    }
}

impl PduEncode for DisplayControlMonitorLayout {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(MonitorLayoutEntry::FIXED_PART_SIZE.try_into().unwrap());

        let monitors_count: u32 = self
            .monitors
            .len()
            .try_into()
            .map_err(|_| invalid_message_err!("NumMonitors", "Number of monitors is too big"))?;

        dst.write_u32(monitors_count);

        for monitor in &self.monitors {
            monitor.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        // As per invariants: This will never overflow:
        // 0 <= Self::FIXED_PART_SIZE + MAX_SUPPORTED_MONITORS * MonitorLayoutEntry::FIXED_PART_SIZE < u16::MAX

        Self::FIXED_PART_SIZE
            .checked_add(self.monitors.iter().map(|monitor| monitor.size()).sum::<usize>())
            .unwrap()
    }
}

impl<'de> PduDecode<'de> for DisplayControlMonitorLayout {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let monitor_layout_size = src.read_u32();

        if monitor_layout_size != MonitorLayoutEntry::FIXED_PART_SIZE.try_into().unwrap() {
            return Err(invalid_message_err!(
                "MonitorLayoutSize",
                "Monitor layout size is invalid"
            ));
        }

        let num_monitors = src.read_u32();

        if num_monitors > MAX_SUPPORTED_MONITORS.into() {
            return Err(invalid_message_err!("NumMonitors", "Too many monitors"));
        }

        let mut monitors = Vec::with_capacity(usize::try_from(num_monitors).unwrap());
        for _ in 0..num_monitors {
            let monitor = MonitorLayoutEntry::decode(src)?;
            monitors.push(monitor);
        }

        Ok(Self { monitors })
    }
}

#[derive(Debug, Clone)]
pub struct MonitorLayoutEntry {
    is_primary: bool,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    physical_width: u32,
    physical_height: u32,
    orientation: u32,
    desktop_scale_factor: u32,
    device_scale_factor: u32,
}

impl MonitorLayoutEntry {
    const FIXED_PART_SIZE: usize = 4 /* Flags */
        + 4 /* Left */
        + 4 /* Top */
        + 4 /* Width */
        + 4 /* Height */
        + 4 /* PhysicalWidth */
        + 4 /* PhysicalHeight */
        + 4 /* Orientation */
        + 4 /* DesktopScaleFactor */
        + 4 /* DeviceScaleFactor */;

    const NAME: &'static str = "DISPLAYCONTROL_MONITOR_LAYOUT";

    fn new_impl(width: u32, height: u32) -> PduResult<Self> {
        validate_dimensions(width, height)?;

        Ok(Self {
            is_primary: false,
            left: 0,
            top: 0,
            width,
            height,
            physical_width: 0,
            physical_height: 0,
            orientation: 0,
            desktop_scale_factor: 100,
            device_scale_factor: 100,
        })
    }

    /// Creates a new primary monitor layout entry.
    pub fn new_primary(width: u32, height: u32) -> PduResult<Self> {
        let mut entry = Self::new_impl(width, height)?;
        entry.is_primary = true;
        Ok(entry)
    }

    /// Creates a new secondary monitor layout entry.
    pub fn new_secondary(width: u32, height: u32) -> PduResult<Self> {
        Self::new_impl(width, height)
    }

    /// Sets the monitor's orientation. (Default is [`MonitorOrientation::Landscape`])
    #[must_use]
    pub fn with_orientation(mut self, orientation: MonitorOrientation) -> Self {
        self.orientation = orientation.angle();
        self
    }

    /// Sets the monitor's position (left, top) in pixels. (Default is (0, 0))
    ///
    /// Note: The primary monitor position must be always (0, 0).
    pub fn with_position(mut self, left: u32, top: u32) -> PduResult<Self> {
        validate_position(left, top, self.is_primary)?;

        self.left = left;
        self.top = top;

        Ok(self)
    }

    /// Sets the monitor's device scale factor in percent. (Default is [`DeviceScaleFactor::Scale100Percent`])
    #[must_use]
    pub fn with_device_scale_factor(mut self, device_scale_factor: DeviceScaleFactor) -> Self {
        self.device_scale_factor = device_scale_factor.value();
        self
    }

    /// Sets the monitor's desktop scale factor in percent. (Default is `100`)
    ///
    /// NOTE: As specified in [MS-RDPEDISP], if the desktop scale factor is not in the valid range
    /// (100..=500 percent), the monitor desktop scale factor is considered invalid and should be ignored.
    pub fn with_desktop_scale_factor(mut self, desktop_scale_factor: u32) -> PduResult<Self> {
        validate_desktop_scale_factor(desktop_scale_factor)?;

        self.desktop_scale_factor = desktop_scale_factor;
        Ok(self)
    }

    /// Sets the monitor's physical dimensions in millimeters.
    ///
    /// NOTE: As specified in [MS-RDPEDISP], if the physical dimensions are not in the valid range
    /// (10..=10000 millimeters), the monitor physical dimensions are considered invalid and
    /// should be ignored.
    pub fn with_physical_dimensions(mut self, physical_width: u32, physical_height: u32) -> PduResult<Self> {
        validate_physical_dimensions(physical_width, physical_height)?;

        self.physical_width = physical_width;
        self.physical_height = physical_height;
        Ok(self)
    }

    pub fn is_primary(&self) -> bool {
        self.is_primary
    }

    /// Returns the monitor's position (left, top) in pixels.
    pub fn position(&self) -> Option<(u32, u32)> {
        validate_position(self.left, self.top, self.is_primary).ok()?;

        Some((self.left, self.top))
    }

    /// Returns the monitor's dimensions (width, height) in pixels.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns the monitor's orientation if it is valid.
    ///
    /// NOTE: As specified in [MS-RDPEDISP], if the orientation is not one of the valid values
    /// (0, 90, 180, 270), the monitor orientation is considered invalid and should be ignored.
    pub fn orientation(&self) -> Option<MonitorOrientation> {
        MonitorOrientation::from_angle(self.orientation)
    }

    /// Returns the monitor's physical dimensions (width, height) in millimeters.
    ///
    /// NOTE: As specified in [MS-RDPEDISP], if the physical dimensions are not in the valid range
    /// (10..=10000 millimeters), the monitor physical dimensions are considered invalid and
    /// should be ignored.
    pub fn physical_dimensions(&self) -> Option<(u32, u32)> {
        validate_physical_dimensions(self.physical_width, self.physical_height).ok()?;
        Some((self.physical_width, self.physical_height))
    }

    /// Returns the monitor's device scale factor in percent if it is valid.
    ///
    /// NOTE: As specified in [MS-RDPEDISP], if the desktop scale factor is not in the valid range
    /// (100..=500 percent), the monitor desktop scale factor is considered invalid and should be ignored.
    ///
    /// IMPORTANT: When processing scale factors, make sure that both desktop and device scale factors
    /// are valid, otherwise they both should be ignored.
    pub fn desktop_scale_factor(&self) -> Option<u32> {
        validate_desktop_scale_factor(self.desktop_scale_factor).ok()?;

        Some(self.desktop_scale_factor)
    }

    /// Returns the monitor's device scale factor in percent if it is valid.
    ///
    /// IMPORTANT: When processing scale factors, make sure that both desktop and device scale factors
    /// are valid, otherwise they both should be ignored.
    pub fn device_scale_factor(&self) -> Option<DeviceScaleFactor> {
        match self.device_scale_factor {
            100 => Some(DeviceScaleFactor::Scale100Percent),
            140 => Some(DeviceScaleFactor::Scale140Percent),
            180 => Some(DeviceScaleFactor::Scale180Percent),
            _ => None,
        }
    }
}

impl PduEncode for MonitorLayoutEntry {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let flags = if self.is_primary {
            DISPLAYCONTROL_MONITOR_PRIMARY
        } else {
            0
        };
        dst.write_u32(flags);
        dst.write_u32(self.left);
        dst.write_u32(self.top);
        dst.write_u32(self.width);
        dst.write_u32(self.height);
        dst.write_u32(self.physical_width);
        dst.write_u32(self.physical_height);
        dst.write_u32(self.orientation);
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

impl<'de> PduDecode<'de> for MonitorLayoutEntry {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = src.read_u32();
        let left = src.read_u32();
        let top = src.read_u32();
        let width = src.read_u32();
        let height = src.read_u32();
        let physical_width = src.read_u32();
        let physical_height = src.read_u32();
        let orientation = src.read_u32();
        let desktop_scale_factor = src.read_u32();
        let device_scale_factor = src.read_u32();

        validate_dimensions(width, height)?;

        Ok(Self {
            is_primary: flags & DISPLAYCONTROL_MONITOR_PRIMARY != 0,
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

/// Valid monitor orientations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorOrientation {
    Landscape,
    Portrait,
    LandscapeFlipped,
    PortraitFlipped,
}

impl MonitorOrientation {
    pub fn from_angle(angle: u32) -> Option<Self> {
        match angle {
            0 => Some(Self::Landscape),
            90 => Some(Self::Portrait),
            180 => Some(Self::LandscapeFlipped),
            270 => Some(Self::PortraitFlipped),
            _ => None,
        }
    }

    pub fn angle(&self) -> u32 {
        match self {
            Self::Landscape => 0,
            Self::Portrait => 90,
            Self::LandscapeFlipped => 180,
            Self::PortraitFlipped => 270,
        }
    }
}

/// Valid device scale factors for monitors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceScaleFactor {
    Scale100Percent,
    Scale140Percent,
    Scale180Percent,
}

impl DeviceScaleFactor {
    pub fn value(&self) -> u32 {
        match self {
            Self::Scale100Percent => 100,
            Self::Scale140Percent => 140,
            Self::Scale180Percent => 180,
        }
    }
}

fn validate_position(left: u32, top: u32, is_primary: bool) -> PduResult<()> {
    if is_primary && (left != 0 || top != 0) {
        return Err(invalid_message_err!(
            "Position",
            "Primary monitor position must be (0, 0)"
        ));
    }

    Ok(())
}

fn validate_dimensions(width: u32, height: u32) -> PduResult<()> {
    if !(200..=8192).contains(&width) {
        return Err(invalid_message_err!("Width", "Monitor width is out of range"));
    }
    if width % 2 != 0 {
        return Err(invalid_message_err!("Width", "Monitor width cannot be odd"));
    }
    if !(200..=8192).contains(&height) {
        return Err(invalid_message_err!("Height", "Monitor height is out of range"));
    }

    Ok(())
}

fn validate_desktop_scale_factor(desktop_scale_factor: u32) -> PduResult<()> {
    if !(100..=500).contains(&desktop_scale_factor) {
        return Err(invalid_message_err!(
            "DesktopScaleFactor",
            "Desktop scale factor is out of range"
        ));
    }

    Ok(())
}

fn validate_physical_dimensions(physical_width: u32, physical_height: u32) -> PduResult<()> {
    if !(10..=10000).contains(&physical_width) {
        return Err(invalid_message_err!("PhysicalWidth", "Physical width is out of range"));
    }
    if !(10..=10000).contains(&physical_height) {
        return Err(invalid_message_err!(
            "PhysicalHeight",
            "Physical height is out of range"
        ));
    }

    Ok(())
}

fn calculate_monitor_area(
    max_num_monitors: u32,
    max_monitor_area_factor_a: u32,
    max_monitor_area_factor_b: u32,
) -> PduResult<u64> {
    if max_num_monitors > MAX_SUPPORTED_MONITORS.into() {
        return Err(invalid_message_err!("NumMonitors", "Too many monitors"));
    }

    if max_monitor_area_factor_a > MAX_MONITOR_AREA_FACTOR.into()
        || max_monitor_area_factor_b > MAX_MONITOR_AREA_FACTOR.into()
    {
        return Err(invalid_message_err!(
            "MaxMonitorAreaFactor",
            "Invalid monitor area factor"
        ));
    }

    // As per invariants: This multiplication would never overflow.
    // 0 <= MAX_MONITOR_AREA_FACTOR * MAX_MONITOR_AREA_FACTOR * MAX_SUPPORTED_MONITORS <= u64::MAX
    Ok((u64::from(max_monitor_area_factor_a))
        .checked_mul(u64::from(max_monitor_area_factor_b))
        .and_then(|monitor_area| monitor_area.checked_mul(u64::from(max_num_monitors)))
        .unwrap())
}
