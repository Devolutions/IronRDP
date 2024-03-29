use ironrdp_pdu::gcc::*;

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
