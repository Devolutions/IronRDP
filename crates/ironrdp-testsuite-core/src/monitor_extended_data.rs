use std::sync::LazyLock;

use ironrdp_pdu::gcc::{ClientMonitorExtendedData, ExtendedMonitorInfo, MonitorOrientation};

pub const MONITOR_DATA_WITHOUT_MONITORS_BUFFER: [u8; 12] =
    [0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

pub const MONITOR_DATA_WITH_MONITORS_BUFFER: [u8; 52] = [
    0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

pub static MONITOR_DATA_WITHOUT_MONITORS: LazyLock<ClientMonitorExtendedData> =
    LazyLock::new(|| ClientMonitorExtendedData {
        extended_monitors_info: Vec::new(),
    });
pub static MONITOR_DATA_WITH_MONITORS: LazyLock<ClientMonitorExtendedData> =
    LazyLock::new(|| ClientMonitorExtendedData {
        extended_monitors_info: vec![
            ExtendedMonitorInfo {
                physical_width: 0,
                physical_height: 0,
                orientation: MonitorOrientation::Landscape,
                desktop_scale_factor: 0,
                device_scale_factor: 0,
            },
            ExtendedMonitorInfo {
                physical_width: 0,
                physical_height: 0,
                orientation: MonitorOrientation::Landscape,
                desktop_scale_factor: 0,
                device_scale_factor: 0,
            },
        ],
    });
