use lazy_static::lazy_static;

use super::*;

pub const MONITOR_DATA_WITHOUT_MONITORS_BUFFER: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
pub const MONITOR_DATA_WITH_MONITORS_BUFFER: [u8; 48] = [
    0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7f, 0x07, 0x00,
    0x00, 0x37, 0x04, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0xfb, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff,
    0xff, 0xff, 0xff, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

lazy_static! {
    pub static ref MONITOR_DATA_WITHOUT_MONITORS: ClientMonitorData = ClientMonitorData { monitors: Vec::new() };
    pub static ref MONITOR_DATA_WITH_MONITORS: ClientMonitorData = ClientMonitorData {
        monitors: vec![
            Monitor {
                left: 0,
                top: 0,
                right: 1919,
                bottom: 1079,
                flags: MonitorFlags::PRIMARY,
            },
            Monitor {
                left: -1280,
                top: 0,
                right: -1,
                bottom: 1023,
                flags: MonitorFlags::empty(),
            }
        ]
    };
}

#[test]
fn from_buffer_correctly_parses_client_monitor_data_without_monitors() {
    let buffer = MONITOR_DATA_WITHOUT_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *MONITOR_DATA_WITHOUT_MONITORS,
        ClientMonitorData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_monitor_data_with_monitors() {
    let buffer = MONITOR_DATA_WITH_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *MONITOR_DATA_WITH_MONITORS,
        ClientMonitorData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_data_without_monitors() {
    let data = MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer = MONITOR_DATA_WITHOUT_MONITORS_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_data_with_monitors() {
    let data = MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer = MONITOR_DATA_WITH_MONITORS_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_monitor_data_without_monitors() {
    let data = MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer_len = MONITOR_DATA_WITHOUT_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_monitor_data_with_monitors() {
    let data = MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer_len = MONITOR_DATA_WITH_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
