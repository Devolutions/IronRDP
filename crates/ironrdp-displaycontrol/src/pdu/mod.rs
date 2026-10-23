//! Display Update Virtual Channel Extension PDUs  [MS-RDPEDISP][1] implementation.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    invalid_field_err,
};
use ironrdp_dvc::DvcEncode;
use tracing::warn;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayControlPdu {
    Caps(DisplayControlCapabilities),
    MonitorLayout(DisplayControlMonitorLayout),
}

impl DisplayControlPdu {
    const NAME: &'static str = "DISPLAYCONTROL_HEADER";
    const FIXED_PART_SIZE: usize = 4 /* Type */ + 4 /* Length */;
}

impl Encode for DisplayControlPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        let (kind, payload_length) = match self {
            DisplayControlPdu::Caps(caps) => (DISPLAYCONTROL_PDU_TYPE_CAPS, caps.size()),
            DisplayControlPdu::MonitorLayout(layout) => (DISPLAYCONTROL_PDU_TYPE_MONITOR_LAYOUT, layout.size()),
        };

        // This will never overflow as per invariants.
        #[expect(clippy::arithmetic_side_effects)]
        let pdu_size = cast_length!("pdu size", payload_length + Self::FIXED_PART_SIZE, in: dst)?;

        // Write `DISPLAYCONTROL_HEADER` fields.
        dst.write_u32(kind);
        dst.write_u32(pdu_size);

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
        // As per invariants: This will never overflow.
        #[expect(clippy::arithmetic_side_effects)]
        let size = Self::FIXED_PART_SIZE
            + match self {
                DisplayControlPdu::Caps(caps) => caps.size(),
                DisplayControlPdu::MonitorLayout(layout) => layout.size(),
            };

        size
    }
}

impl DvcEncode for DisplayControlPdu {}

impl<'de> Decode<'de> for DisplayControlPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        // Read `DISPLAYCONTROL_HEADER` fields.
        let kind = src.read_u32();
        let pdu_length = src.read_u32();

        let payload_length = pdu_length
            .checked_sub(Self::FIXED_PART_SIZE.try_into().expect("always in range"))
            .ok_or_else(|| invalid_field_err!("Length", "Display control PDU length is too small", in: src))?;

        if usize::try_from(payload_length).unwrap_or(usize::MAX) != src.len() {
            return Err(invalid_field_err!(
                "Length",
                "Display control PDU length does not match the payload size",
                in: src
            ));
        }

        let pdu = match kind {
            DISPLAYCONTROL_PDU_TYPE_CAPS => DisplayControlPdu::Caps(DisplayControlCapabilities::decode(src)?),
            DISPLAYCONTROL_PDU_TYPE_MONITOR_LAYOUT => {
                DisplayControlPdu::MonitorLayout(DisplayControlMonitorLayout::decode(src)?)
            }
            _ => return Err(invalid_field_err!("Type", "Unknown display control PDU type", in: src)),
        };

        if !src.is_empty() {
            return Err(invalid_field_err!(
                "Length",
                "trailing bytes after display control PDU body",
                in: src
            ));
        }

        Ok(pdu)
    }
}

impl From<DisplayControlCapabilities> for DisplayControlPdu {
    fn from(caps: DisplayControlCapabilities) -> Self {
        Self::Caps(caps)
    }
}

impl From<DisplayControlMonitorLayout> for DisplayControlPdu {
    fn from(layout: DisplayControlMonitorLayout) -> Self {
        Self::MonitorLayout(layout)
    }
}

/// 2.2.2.1 DISPLAYCONTROL_CAPS_PDU
///
/// Display control channel capabilities PDU.
///
/// INVARIANTS:
///     0 <= max_num_monitors <= MAX_SUPPORTED_MONITORS
///     0 <= max_monitor_area_factor_a <= MAX_MONITOR_AREA_FACTOR
///     0 <= max_monitor_area_factor_b <= MAX_MONITOR_AREA_FACTOR
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/8989a211-984e-4ecc-80f3-60694fc4b476
#[derive(Debug, Clone, PartialEq, Eq)]
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
    ) -> DecodeResult<Self> {
        let max_monitor_area =
            calculate_monitor_area(max_num_monitors, max_monitor_area_factor_a, max_monitor_area_factor_b)?;

        Ok(Self {
            max_num_monitors,
            max_monitor_area_factor_a,
            max_monitor_area_factor_b,
            max_monitor_area,
        })
    }

    /// Returns the maximum number of monitors the server accepts in a monitor layout (`MaxNumMonitors`).
    pub fn max_num_monitors(&self) -> u32 {
        self.max_num_monitors
    }

    /// Returns the maximum total area, in square pixels, of a monitor layout:
    /// `MaxNumMonitors * MaxMonitorAreaFactorA * MaxMonitorAreaFactorB`.
    pub fn max_monitor_area(&self) -> u64 {
        self.max_monitor_area
    }

    /// One 4K-area (3840x2400) monitor: the single-monitor capabilities every
    /// existing server advertised before per-display monitor counts existed,
    /// and the safe fallback for a monitor count that turns out to be invalid.
    ///
    /// # Panics
    ///
    /// Never: `(1, 3840, 2400)` is always within [`new`](Self::new)'s valid range.
    pub fn single_monitor() -> Self {
        Self::new(1, 3840, 2400).expect("(1, 3840, 2400) are always within the valid range")
    }
}

impl Encode for DisplayControlCapabilities {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
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

impl<'de> Decode<'de> for DisplayControlCapabilities {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
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

/// [2.2.2.2] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// Sent from client to server to notify about new monitor layout (e.g screen resize).
///
/// INVARIANTS:
///     0 <= monitors.length() <= MAX_SUPPORTED_MONITORS
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/22741217-12a0-4fb8-b5a0-df43905aaf06
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayControlMonitorLayout {
    monitors: Vec<MonitorLayoutEntry>,
}

impl DisplayControlMonitorLayout {
    const NAME: &'static str = "DISPLAYCONTROL_MONITOR_LAYOUT_PDU";
    const FIXED_PART_SIZE: usize = 4 /* MonitorLayoutSize */ + 4 /* NumMonitors */;

    /// Creates a new [`DisplayControlMonitorLayout`] from the complete client monitor layout.
    ///
    /// Per [2.2.2.2], [2.2.2.2.1] and [3.1.5.2], the layout is rejected unless:
    /// - it has at most 1024 monitors, exactly one of which is primary;
    /// - the primary monitor is at (0, 0) (enforced by [`MonitorLayoutEntry::with_position`]);
    /// - no two monitors overlap;
    /// - when there is more than one monitor, each monitor is adjacent to at least one other
    ///   monitor, even if only at a single point.
    ///
    /// Use [`Self::fit_to_capabilities`] to adapt the layout to the limits the server advertised
    /// in its [`DisplayControlCapabilities`].
    ///
    /// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/22741217-12a0-4fb8-b5a0-df43905aaf06
    /// [2.2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    /// [3.1.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9
    pub fn new(monitors: &[MonitorLayoutEntry]) -> EncodeResult<Self> {
        if monitors.len() > MAX_SUPPORTED_MONITORS.into() {
            return Err(invalid_field_err!("NumMonitors", "Too many monitors"));
        }

        let primary_monitors_count = monitors.iter().filter(|monitor| monitor.is_primary()).count();

        if primary_monitors_count != 1 {
            return Err(invalid_field_err!(
                "PrimaryMonitor",
                "There must be exactly one primary monitor"
            ));
        }

        for (index, monitor) in monitors.iter().enumerate() {
            if monitors[..index].iter().any(|other| monitor.overlaps(other)) {
                return Err(invalid_field_err!("Monitors", "monitors must not overlap"));
            }
        }

        if 1 < monitors.len() {
            for (index, monitor) in monitors.iter().enumerate() {
                let is_adjacent = monitors
                    .iter()
                    .enumerate()
                    .any(|(other_index, other)| other_index != index && monitor.touches(other));
                if !is_adjacent {
                    return Err(invalid_field_err!(
                        "Monitors",
                        "each monitor must be adjacent to another monitor"
                    ));
                }
            }
        }

        Ok(Self {
            monitors: monitors.to_vec(),
        })
    }

    /// Adapts this layout to the limits the server advertised in its [`DisplayControlCapabilities`].
    ///
    /// Per [3.2.5.2], the client must not exceed `MaxNumMonitors` nor the maximum monitor area
    /// (`MaxNumMonitors * MaxMonitorAreaFactorA * MaxMonitorAreaFactorB`):
    /// - monitors beyond `MaxNumMonitors` are dropped: starting from the primary monitor, the
    ///   monitors adjacent to an already kept monitor are kept first, in their original order,
    ///   so the reduced layout stays connected;
    /// - a single-monitor layout larger than the maximum area is scaled down, preserving its
    ///   aspect ratio;
    /// - a multi-monitor layout larger than the maximum area is rejected, since shrinking it
    ///   would move the monitors relative to each other.
    ///
    /// An error means no layout within the server limits preserves the request; callers should
    /// keep the current layout rather than send a partial one.
    ///
    /// [3.2.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9
    pub fn fit_to_capabilities(&self, capabilities: &DisplayControlCapabilities) -> EncodeResult<Self> {
        let max_num_monitors = usize::try_from(capabilities.max_num_monitors()).unwrap_or(usize::MAX);
        if max_num_monitors == 0 {
            return Err(invalid_field_err!("NumMonitors", "server accepts no monitors"));
        }

        let mut monitors = if max_num_monitors < self.monitors.len() {
            warn!(
                requested = self.monitors.len(),
                max_num_monitors, "Dropping monitors beyond the server maximum"
            );
            // Grow the kept set from the primary monitor, one adjacent monitor at a time, so the
            // reduced layout stays connected whatever order the monitors are listed in.
            let mut kept = self
                .monitors
                .iter()
                .map(MonitorLayoutEntry::is_primary)
                .collect::<Vec<_>>();
            let mut kept_count = kept.iter().filter(|kept| **kept).count();
            while kept_count < max_num_monitors {
                let next = self.monitors.iter().enumerate().position(|(index, monitor)| {
                    !kept[index]
                        && self
                            .monitors
                            .iter()
                            .zip(&kept)
                            .any(|(other, other_kept)| *other_kept && monitor.touches(other))
                });
                let Some(next) = next else {
                    break;
                };
                kept[next] = true;
                kept_count = kept_count.saturating_add(1);
            }
            self.monitors
                .iter()
                .zip(kept)
                .filter(|(_, kept)| *kept)
                .map(|(monitor, _)| monitor.clone())
                .collect::<Vec<_>>()
        } else {
            self.monitors.clone()
        };

        let max_area = capabilities.max_monitor_area();
        if max_area < layout_area(&monitors) {
            let [monitor] = monitors.as_mut_slice() else {
                return Err(invalid_field_err!(
                    "Monitors",
                    "monitor layout area exceeds the server maximum"
                ));
            };

            let (width, height) = (u128::from(monitor.width), u128::from(monitor.height));
            // Largest width w' such that w' * (w' * height / width) <= max_area, i.e.
            // w' = sqrt(width^2 * max_area / (width * height)).
            // No overflow: width, height <= 8192 and max_area < 2^39, so every product fits in u128.
            #[expect(clippy::arithmetic_side_effects)]
            let (new_width, new_height) = {
                let new_width = (width * width * u128::from(max_area) / (width * height)).isqrt();
                let new_height = new_width * height / width;
                (new_width, new_height)
            };
            let (new_width, new_height) = MonitorLayoutEntry::adjust_display_size(
                u32::try_from(new_width).unwrap_or(u32::MAX),
                u32::try_from(new_height).unwrap_or(u32::MAX),
            );
            if max_area < u64::from(new_width).saturating_mul(u64::from(new_height)) {
                return Err(invalid_field_err!(
                    "Monitors",
                    "server maximum monitor area is smaller than the minimum monitor size"
                ));
            }

            warn!(
                from_width = monitor.width,
                from_height = monitor.height,
                to_width = new_width,
                to_height = new_height,
                max_area,
                "Scaling the monitor down to the server maximum area"
            );
            monitor.width = new_width;
            monitor.height = new_height;
        }

        Self::new(&monitors)
    }

    /// Returns the size (width, height), in pixels, of the smallest rectangle containing every monitor.
    pub fn desktop_size(&self) -> (u32, u32) {
        let (left, top, right, bottom) = self.monitors.iter().map(MonitorLayoutEntry::bounds).fold(
            (i64::MAX, i64::MAX, i64::MIN, i64::MIN),
            |(left, top, right, bottom), (monitor_left, monitor_top, monitor_right, monitor_bottom)| {
                (
                    left.min(monitor_left),
                    top.min(monitor_top),
                    right.max(monitor_right),
                    bottom.max(monitor_bottom),
                )
            },
        );
        (
            u32::try_from(right.saturating_sub(left)).unwrap_or(0),
            u32::try_from(bottom.saturating_sub(top)).unwrap_or(0),
        )
    }

    /// Creates a new [`DisplayControlMonitorLayout`] with a single primary monitor
    ///
    /// Per [2.2.2.2.1]:
    /// - The `width` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels, and MUST NOT be an odd value.
    /// - The `height` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels.
    /// - The `scale_factor` MUST be ignored if it is less than 100 percent or greater than 500 percent.
    /// - The `physical_dims` (width, height) MUST be ignored if either is less than 10 mm or greater than 10,000 mm.
    ///
    /// The desktop `scale_factor` is clamped with [`MonitorLayoutEntry::adjust_desktop_scale_factor`], and
    /// the device scale factor is derived from it with [`DeviceScaleFactor::from_desktop_scale_factor`].
    /// The orientation is [`MonitorOrientation::Portrait`] when `height` exceeds `width`, and
    /// [`MonitorOrientation::Landscape`] otherwise; use [`Self::new`] for an explicit orientation.
    ///
    /// Use [`MonitorLayoutEntry::adjust_display_size`] to adjust `width` and `height` before calling this function
    /// to ensure the display size is within the valid range.
    ///
    /// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn new_single_primary_monitor(
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_dims: Option<(u32, u32)>,
    ) -> EncodeResult<Self> {
        let entry = MonitorLayoutEntry::new_primary(width, height)?.with_orientation(if width < height {
            MonitorOrientation::Portrait
        } else {
            MonitorOrientation::Landscape
        });

        let entry = if let Some(scale_factor) = scale_factor {
            let desktop_scale_factor = MonitorLayoutEntry::adjust_desktop_scale_factor(scale_factor);
            entry
                .with_desktop_scale_factor(desktop_scale_factor)?
                .with_device_scale_factor(DeviceScaleFactor::from_desktop_scale_factor(desktop_scale_factor))
        } else {
            entry
        };

        let entry = if let Some((physical_width, physical_height)) = physical_dims {
            entry.with_physical_dimensions(physical_width, physical_height)?
        } else {
            entry
        };

        DisplayControlMonitorLayout::new(&[entry])
    }

    pub fn monitors(&self) -> &[MonitorLayoutEntry] {
        &self.monitors
    }
}

impl Encode for DisplayControlMonitorLayout {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(MonitorLayoutEntry::FIXED_PART_SIZE.try_into().expect("always in range"));

        let monitors_count: u32 = self
            .monitors
            .len()
            .try_into()
            .map_err(|_| invalid_field_err!("NumMonitors", "Number of monitors is too big", in: dst))?;

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
        #[expect(clippy::arithmetic_side_effects)]
        let size = Self::FIXED_PART_SIZE + self.monitors.iter().map(|monitor| monitor.size()).sum::<usize>();

        size
    }
}

impl<'de> Decode<'de> for DisplayControlMonitorLayout {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let monitor_layout_size = src.read_u32();

        if monitor_layout_size != MonitorLayoutEntry::FIXED_PART_SIZE.try_into().expect("always in range") {
            return Err(invalid_field_err!( "MonitorLayoutSize",
                "Monitor layout size is invalid", in: src));
        }

        let num_monitors = cast_length!("number of monitors", src.read_u32(), in: src)?;

        if num_monitors > MAX_SUPPORTED_MONITORS.into() {
            return Err(invalid_field_err!("NumMonitors", "Too many monitors", in: src));
        }

        let mut monitors = Vec::with_capacity(num_monitors);
        for _ in 0..num_monitors {
            let monitor = MonitorLayoutEntry::decode(src)?;
            monitors.push(monitor);
        }

        Ok(Self { monitors })
    }
}

/// [2.2.2.2.1] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorLayoutEntry {
    is_primary: bool,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    physical_width: u32,
    physical_height: u32,
    orientation: u32,
    desktop_scale_factor: u32,
    device_scale_factor: u32,
}

/// Takes `at:` only where a stream position exists.
///
/// A constructor validating its arguments has no input stream, so it omits the
/// offset rather than passing zero: `Some(0)` would render ` at offset 0` and
/// name a position the caller never read from.
macro_rules! validate_dimensions {
    ($width:expr, $height:expr $(, at: $offset:expr)?) => {{
        if !(200..=8192).contains(&$width) {
            return Err(invalid_field_err!("Width", "Monitor width is out of range" $(, at: $offset)?));
        }
        if $width % 2 != 0 {
            return Err(invalid_field_err!("Width", "Monitor width cannot be odd" $(, at: $offset)?));
        }
        if !(200..=8192).contains(&$height) {
            return Err(invalid_field_err!("Height", "Monitor height is out of range" $(, at: $offset)?));
        }
        Ok(())
    }};
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

    /// Creates a new [`MonitorLayoutEntry`].
    ///
    /// Per [2.2.2.2.1]:
    /// - The `width` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels, and MUST NOT be an odd value.
    /// - The `height` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels.
    ///
    /// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    fn new_impl(mut width: u32, height: u32) -> EncodeResult<Self> {
        if !width.is_multiple_of(2) {
            let prev_width = width;
            width = width.saturating_sub(1);
            warn!(
                "Monitor width cannot be odd, adjusting from [{}] to [{}]",
                prev_width, width
            )
        }

        validate_dimensions!(width, height)?;

        Ok(Self {
            is_primary: false,
            left: 0,
            top: 0,
            width,
            height,
            physical_width: 0,
            physical_height: 0,
            orientation: 0,
            desktop_scale_factor: 0,
            device_scale_factor: 0,
        })
    }

    /// Adjusts the display size to be within the valid range.
    ///
    /// Per [2.2.2.2.1]:
    /// - The `width` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels, and MUST NOT be an odd value.
    /// - The `height` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels.
    ///
    /// Functions that create [`MonitorLayoutEntry`] should typically use this function to adjust the display size first.
    ///
    /// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn adjust_display_size(width: u32, height: u32) -> (u32, u32) {
        fn constrain(value: u32) -> u32 {
            value.clamp(200, 8192)
        }

        let mut width = width;
        if !width.is_multiple_of(2) {
            width = width.saturating_sub(1);
        }

        (constrain(width), constrain(height))
    }

    /// Clamps a desktop scale factor, in percent, to the valid range.
    ///
    /// Per [2.2.2.2.1], the desktop scale factor MUST be ignored if it is less than 100 percent
    /// or greater than 500 percent.
    ///
    /// [2.2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn adjust_desktop_scale_factor(desktop_scale_factor: u32) -> u32 {
        desktop_scale_factor.clamp(100, 500)
    }

    /// Creates a new primary [`MonitorLayoutEntry`].
    ///
    /// Per [2.2.2.2.1]:
    /// - The `width` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels, and MUST NOT be an odd value.
    /// - The `height` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels.
    ///
    /// Use [`MonitorLayoutEntry::adjust_display_size`] before calling this function to ensure the display size is within the valid range.
    ///
    /// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn new_primary(width: u32, height: u32) -> EncodeResult<Self> {
        let mut entry = Self::new_impl(width, height)?;
        entry.is_primary = true;
        Ok(entry)
    }

    /// Creates a new primary [`MonitorLayoutEntry`].
    ///
    /// Per [2.2.2.2.1]:
    /// - The `width` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels, and MUST NOT be an odd value.
    /// - The `height` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels.
    ///
    /// Use [`MonitorLayoutEntry::adjust_display_size`] before calling this function to ensure the display size is within the valid range.
    ///
    /// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn new_secondary(width: u32, height: u32) -> EncodeResult<Self> {
        Self::new_impl(width, height)
    }

    /// Sets the monitor's orientation. (Default is [`MonitorOrientation::Landscape`])
    ///
    /// The width and height of the entry describe the desktop as the user sees it, after the
    /// rotation: a 1080x1920 portrait monitor has a width of 1080.
    #[must_use]
    pub fn with_orientation(mut self, orientation: MonitorOrientation) -> Self {
        self.orientation = orientation.angle();
        self
    }

    /// Sets the monitor's position (left, top) in pixels. (Default is (0, 0))
    ///
    /// Note: The primary monitor position must be always (0, 0).
    pub fn with_position(mut self, left: i32, top: i32) -> EncodeResult<Self> {
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

    /// Sets the monitor's desktop scale factor in percent.
    ///
    /// NOTE: As specified in [MS-RDPEDISP], if the desktop scale factor is not in the valid range
    /// (100..=500 percent), the monitor desktop scale factor is considered invalid and should be ignored.
    pub fn with_desktop_scale_factor(mut self, desktop_scale_factor: u32) -> EncodeResult<Self> {
        validate_desktop_scale_factor(desktop_scale_factor)?;

        self.desktop_scale_factor = desktop_scale_factor;
        Ok(self)
    }

    /// Sets the monitor's physical dimensions in millimeters.
    ///
    /// NOTE: As specified in [MS-RDPEDISP], if the physical dimensions are not in the valid range
    /// (10..=10000 millimeters), the monitor physical dimensions are considered invalid and
    /// should be ignored.
    pub fn with_physical_dimensions(mut self, physical_width: u32, physical_height: u32) -> EncodeResult<Self> {
        validate_physical_dimensions(physical_width, physical_height)?;

        self.physical_width = physical_width;
        self.physical_height = physical_height;
        Ok(self)
    }

    pub fn is_primary(&self) -> bool {
        self.is_primary
    }

    /// Returns the monitor's position (left, top) in pixels.
    pub fn position(&self) -> Option<(i32, i32)> {
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
        DeviceScaleFactor::from_value(self.device_scale_factor)
    }

    /// Returns the monitor rectangle as (left, top, right, bottom), right and bottom exclusive.
    fn bounds(&self) -> (i64, i64, i64, i64) {
        let left = i64::from(self.left);
        let top = i64::from(self.top);
        // No overflow: an i32 plus a u32 always fits in an i64.
        #[expect(clippy::arithmetic_side_effects)]
        let (right, bottom) = (left + i64::from(self.width), top + i64::from(self.height));
        (left, top, right, bottom)
    }

    /// Returns whether both monitors share at least one pixel.
    fn overlaps(&self, other: &Self) -> bool {
        let (left, top, right, bottom) = self.bounds();
        let (other_left, other_top, other_right, other_bottom) = other.bounds();
        left < other_right && other_left < right && top < other_bottom && other_top < bottom
    }

    /// Returns whether both monitors overlap or share an edge or a corner.
    fn touches(&self, other: &Self) -> bool {
        let (left, top, right, bottom) = self.bounds();
        let (other_left, other_top, other_right, other_bottom) = other.bounds();
        left <= other_right && other_left <= right && top <= other_bottom && other_top <= bottom
    }
}

impl Encode for MonitorLayoutEntry {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        let flags = if self.is_primary {
            DISPLAYCONTROL_MONITOR_PRIMARY
        } else {
            0
        };
        dst.write_u32(flags);
        dst.write_i32(self.left);
        dst.write_i32(self.top);
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

impl<'de> Decode<'de> for MonitorLayoutEntry {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = src.read_u32();
        let left = src.read_i32();
        let top = src.read_i32();
        let width = src.read_u32();
        let height = src.read_u32();
        let physical_width = src.read_u32();
        let physical_height = src.read_u32();
        let orientation = src.read_u32();
        let desktop_scale_factor = src.read_u32();
        let device_scale_factor = src.read_u32();

        validate_dimensions!(width, height, at: src.pos())?;

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
    /// Returns the device scale factor for a value in percent, if it is one of 100, 140 or 180.
    pub fn from_value(value: u32) -> Option<Self> {
        match value {
            100 => Some(Self::Scale100Percent),
            140 => Some(Self::Scale140Percent),
            180 => Some(Self::Scale180Percent),
            _ => None,
        }
    }

    /// Returns the device scale factor matching a desktop scale factor in percent.
    ///
    /// The device scale factor is the plateau the desktop scale factor reaches: below 140 percent
    /// maps to 100 percent, below 180 percent maps to 140 percent, and anything higher maps to
    /// 180 percent.
    pub fn from_desktop_scale_factor(desktop_scale_factor: u32) -> Self {
        if desktop_scale_factor < 140 {
            Self::Scale100Percent
        } else if desktop_scale_factor < 180 {
            Self::Scale140Percent
        } else {
            Self::Scale180Percent
        }
    }

    pub fn value(&self) -> u32 {
        match self {
            Self::Scale100Percent => 100,
            Self::Scale140Percent => 140,
            Self::Scale180Percent => 180,
        }
    }
}

fn layout_area(monitors: &[MonitorLayoutEntry]) -> u64 {
    monitors.iter().fold(0, |area, monitor| {
        area.saturating_add(u64::from(monitor.width).saturating_mul(u64::from(monitor.height)))
    })
}

fn validate_position(left: i32, top: i32, is_primary: bool) -> EncodeResult<()> {
    if is_primary && (left != 0 || top != 0) {
        return Err(invalid_field_err!(
            "Position",
            "Primary monitor position must be (0, 0)"
        ));
    }

    Ok(())
}

fn validate_desktop_scale_factor(desktop_scale_factor: u32) -> EncodeResult<()> {
    if !(100..=500).contains(&desktop_scale_factor) {
        return Err(invalid_field_err!(
            "DesktopScaleFactor",
            "Desktop scale factor is out of range"
        ));
    }

    Ok(())
}

fn validate_physical_dimensions(physical_width: u32, physical_height: u32) -> EncodeResult<()> {
    if !(10..=10000).contains(&physical_width) {
        return Err(invalid_field_err!("PhysicalWidth", "Physical width is out of range"));
    }
    if !(10..=10000).contains(&physical_height) {
        return Err(invalid_field_err!("PhysicalHeight", "Physical height is out of range"));
    }

    Ok(())
}

fn calculate_monitor_area(
    max_num_monitors: u32,
    max_monitor_area_factor_a: u32,
    max_monitor_area_factor_b: u32,
) -> DecodeResult<u64> {
    if max_num_monitors > MAX_SUPPORTED_MONITORS.into() {
        return Err(invalid_field_err!("NumMonitors", "Too many monitors"));
    }

    if max_monitor_area_factor_a > MAX_MONITOR_AREA_FACTOR.into()
        || max_monitor_area_factor_b > MAX_MONITOR_AREA_FACTOR.into()
    {
        return Err(invalid_field_err!(
            "MaxMonitorAreaFactor",
            "Invalid monitor area factor"
        ));
    }

    // As per invariants: This multiplication would never overflow.
    // 0 <= MAX_MONITOR_AREA_FACTOR * MAX_MONITOR_AREA_FACTOR * MAX_SUPPORTED_MONITORS <= u64::MAX
    #[expect(clippy::arithmetic_side_effects)]
    Ok(u64::from(max_monitor_area_factor_a) * u64::from(max_monitor_area_factor_b) * u64::from(max_num_monitors))
}
