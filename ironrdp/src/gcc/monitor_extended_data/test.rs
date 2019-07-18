use lazy_static::lazy_static;

use super::*;

pub const MONITOR_DATA_WITHOUT_MONITORS_BUFFER: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
pub const MONITOR_DATA_WITH_MONITORS_BUFFER: [u8; 52] = [
    0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
];

lazy_static! {
    pub static ref MONITOR_DATA_WITHOUT_MONITORS: ClientMonitorExtendedData =
        ClientMonitorExtendedData {
            extended_monitors_info: Vec::new()
        };
    pub static ref MONITOR_DATA_WITH_MONITORS: ClientMonitorExtendedData =
        ClientMonitorExtendedData {
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
                }
            ]
        };
}

#[test]
fn from_buffer_correctly_parses_client_monitor_extended_data_without_monitors() {
    let buffer = MONITOR_DATA_WITHOUT_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *MONITOR_DATA_WITHOUT_MONITORS,
        ClientMonitorExtendedData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_monitor_extended_data_with_monitors() {
    let buffer = MONITOR_DATA_WITH_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *MONITOR_DATA_WITH_MONITORS,
        ClientMonitorExtendedData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_extended_data_without_monitors() {
    let data = MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer = MONITOR_DATA_WITHOUT_MONITORS_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_extended_data_with_monitors() {
    let data = MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer = MONITOR_DATA_WITH_MONITORS_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_monitor_extended_data_without_monitors() {
    let data = MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer_len = MONITOR_DATA_WITHOUT_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_monitor_extended_data_with_monitors() {
    let data = MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer_len = MONITOR_DATA_WITH_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
