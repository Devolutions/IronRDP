use lazy_static::lazy_static;

use super::{finalization_messages::*, headers::*, *};
use crate::{
    gcc::{self, monitor_data},
    rdp::{
        capability_sets::test::{
            CLIENT_DEMAND_ACTIVE, CLIENT_DEMAND_ACTIVE_BUFFER, SERVER_DEMAND_ACTIVE,
            SERVER_DEMAND_ACTIVE_BUFFER,
        },
        client_info::test::{CLIENT_INFO_BUFFER_UNICODE, CLIENT_INFO_UNICODE},
        client_license::test::{LICENSE_PACKET, LICENSE_PACKET_BUFFER},
    },
};

const CLIENT_INFO_PDU_SECURITY_HEADER_BUFFER: [u8; 4] = [
    0x40, 0x00, // flags
    0x00, 0x00, // flagsHi
];
const CLIENT_LICENSE_PDU_SECURITY_HEADER_BUFFER: [u8; 4] = [
    0x80, 0x00, // flags
    0x00, 0x00, // flagsHi
];
const SERVER_DEMAND_ACTIVE_PDU_HEADERS_BUFFER: [u8; 10] = [
    0x6f, 0x01, // ShareControlHeader::totalLength
    0x11, 0x00, // ShareControlHeader::pduType
    0xea, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
];
const CLIENT_DEMAND_ACTIVE_PDU_HEADERS_BUFFER: [u8; 10] = [
    0xf0, 0x01, // ShareControlHeader::totalLength
    0x13, 0x00, // ShareControlHeader::pduType
    0xef, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
];
const MONITOR_LAYOUT_HEADERS_BUFFER: [u8; 18] = [
    0x3e, 0x00, // ShareControlHeader::totalLength
    0x17, 0x00, // ShareControlHeader::pduType
    0xef, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
    0x00, // padding
    0x01, // stream id
    0x30, 0x00, // uncompressed length
    0x37, // pdu type
    0x00, // compression type
    0x00, 0x00, // compressed length
];
const CLIENT_SYNCHRONIZE_BUFFER: [u8; 22] = [
    0x16, 0x00, // ShareControlHeader::totalLength
    0x17, 0x00, // ShareControlHeader::pduType
    0xef, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
    0x00, // padding
    0x01, // stream id
    0x08, 0x00, // uncompressed length
    0x1f, // pdu type
    0x00, // compression type
    0x00, 0x00, // compressed length
    0x01, 0x00, // message type
    0xea, 0x03, // target user
];
const CONTROL_COOPERATE_BUFFER: [u8; 26] = [
    0x1a, 0x00, // ShareControlHeader::totalLength
    0x17, 0x00, // ShareControlHeader::pduType
    0xef, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
    0x00, // padding
    0x01, // stream id
    0x0c, 0x00, // uncompressed length
    0x14, // pdu type
    0x00, // compression type
    0x00, 0x00, // compressed length
    0x04, 0x00, // action
    0x00, 0x00, // grant id
    0x00, 0x00, 0x00, 0x00, // control id
];
const CONTROL_REQUEST_CONTROL_BUFFER: [u8; 26] = [
    0x1a, 0x00, // ShareControlHeader::totalLength
    0x17, 0x00, // ShareControlHeader::pduType
    0xef, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
    0x00, // padding
    0x01, // stream id
    0x0c, 0x00, // uncompressed length
    0x14, // pdu type
    0x00, // compression type
    0x00, 0x00, // compressed length
    0x01, 0x00, // action
    0x00, 0x00, // grant id
    0x00, 0x00, 0x00, 0x00, // control id
];
const SERVER_GRANTED_CONTROL_BUFFER: [u8; 26] = [
    0x1a, 0x00, // ShareControlHeader::totalLength
    0x17, 0x00, // ShareControlHeader::pduType
    0xea, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
    0x00, // padding
    0x02, // stream id
    0x0c, 0x00, // uncompressed length
    0x14, // pdu type
    0x00, // compression type
    0x00, 0x00, // compressed length
    0x02, 0x00, // action
    0xef, 0x03, // grant id
    0xea, 0x03, 0x00, 0x00, // control id
];
const CLIENT_FONT_LIST_BUFFER: [u8; 26] = [
    0x1a, 0x00, // ShareControlHeader::totalLength
    0x17, 0x00, // ShareControlHeader::pduType
    0xef, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
    0x00, // padding
    0x01, // stream id
    0x0c, 0x00, // uncompressed length
    0x27, // pdu type
    0x00, // compression type
    0x00, 0x00, // compressed length
    0x00, 0x00, // number entries
    0x00, 0x00, // total number entries
    0x03, 0x00, // list flags
    0x32, 0x00, // entry size
];
const SERVER_FONT_MAP_BUFFER: [u8; 26] = [
    0x1a, 0x00, // ShareControlHeader::totalLength
    0x17, 0x00, // ShareControlHeader::pduType
    0xea, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
    0x00, // padding
    0x02, // stream id
    0x0c, 0x00, // uncompressed length
    0x28, // pdu type
    0x00, // compression type
    0x00, 0x00, // compressed length
    0x00, 0x00, // number entries
    0x00, 0x00, // total number entries
    0x03, 0x00, // list flags
    0x04, 0x00, // entry size
];

lazy_static! {
    pub static ref CLIENT_INFO_PDU: ClientInfoPdu = ClientInfoPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::INFO_PKT,
        },
        client_info: CLIENT_INFO_UNICODE.clone(),
    };
    pub static ref CLIENT_LICENSE_PDU: ClientLicensePdu = ClientLicensePdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::LICENSE_PKT,
        },
        client_license: LICENSE_PACKET.clone(),
    };
    pub static ref SERVER_DEMAND_ACTIVE_PDU: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()),
        pdu_version: 16,
        pdu_source: 1002,
        share_id: 66_538,
    };
    pub static ref CLIENT_DEMAND_ACTIVE_PDU: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::ClientConfirmActive(CLIENT_DEMAND_ACTIVE.clone()),
        pdu_version: 16,
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref CLIENT_SYNCHRONIZE: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::Synchronize(SynchronizePdu {
                target_user_id: 0x03ea
            }),
            stream_priority: StreamPriority::Low,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_version: 16,
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref CONTROL_COOPERATE: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::Control(ControlPdu {
                action: ControlAction::Cooperate,
                grant_id: 0,
                control_id: 0,
            }),
            stream_priority: StreamPriority::Low,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_version: 16,
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref CONTROL_REQUEST_CONTROL: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::Control(ControlPdu {
                action: ControlAction::RequestControl,
                grant_id: 0,
                control_id: 0,
            }),
            stream_priority: StreamPriority::Low,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_version: 16,
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref SERVER_GRANTED_CONTROL: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::Control(ControlPdu {
                action: ControlAction::GrantedControl,
                grant_id: 1007,
                control_id: 1002,
            }),
            stream_priority: StreamPriority::Medium,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_version: 16,
        pdu_source: 1002,
        share_id: 66_538,
    };
    pub static ref CLIENT_FONT_LIST: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::FontList(FontPdu {
                number: 0,
                total_number: 0,
                flags: SequenceFlags::FIRST | SequenceFlags::LAST,
                entry_size: 50,
            }),
            stream_priority: StreamPriority::Low,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_version: 16,
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref SERVER_FONT_MAP: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::FontMap(FontPdu {
                number: 0,
                total_number: 0,
                flags: SequenceFlags::FIRST | SequenceFlags::LAST,
                entry_size: 4,
            }),
            stream_priority: StreamPriority::Medium,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_version: 16,
        pdu_source: 1002,
        share_id: 66_538,
    };
    pub static ref MONITOR_LAYOUT_PDU: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::MonitorLayout(MonitorLayoutPdu {
                monitors: gcc::monitor_data::test::MONITOR_DATA_WITH_MONITORS
                    .monitors
                    .clone(),
            }),
            stream_priority: StreamPriority::Low,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_version: 16,
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref CLIENT_INFO_PDU_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_INFO_PDU_SECURITY_HEADER_BUFFER.to_vec();
        buffer.extend(CLIENT_INFO_BUFFER_UNICODE.as_ref());

        buffer
    };
    pub static ref CLIENT_LICENSE_PDU_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_LICENSE_PDU_SECURITY_HEADER_BUFFER.to_vec();
        buffer.extend(LICENSE_PACKET_BUFFER.as_ref());

        buffer
    };
    pub static ref SERVER_DEMAND_ACTIVE_PDU_BUFFER: Vec<u8> = {
        let mut buffer = SERVER_DEMAND_ACTIVE_PDU_HEADERS_BUFFER.to_vec();
        buffer.extend(SERVER_DEMAND_ACTIVE_BUFFER.as_ref());

        buffer
    };
    pub static ref CLIENT_DEMAND_ACTIVE_PDU_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_DEMAND_ACTIVE_PDU_HEADERS_BUFFER.to_vec();
        buffer.extend(CLIENT_DEMAND_ACTIVE_BUFFER.as_ref());

        buffer
    };
    pub static ref MONITOR_LAYOUT_PDU_BUFFER: Vec<u8> = {
        let mut buffer = MONITOR_LAYOUT_HEADERS_BUFFER.to_vec();
        buffer.extend(
            monitor_data::test::MONITOR_DATA_WITH_MONITORS_BUFFER
                .to_vec()
                .split_off(gcc::MONITOR_FLAGS_SIZE),
        );

        buffer
    };
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_info() {
    let buf = CLIENT_INFO_PDU_BUFFER.clone();

    assert_eq!(
        CLIENT_INFO_PDU.clone(),
        ClientInfoPdu::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_license() {
    let buf = CLIENT_LICENSE_PDU_BUFFER.clone();

    assert_eq!(
        CLIENT_LICENSE_PDU.clone(),
        ClientLicensePdu::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_server_demand_active() {
    let buf = SERVER_DEMAND_ACTIVE_PDU_BUFFER.clone();

    assert_eq!(
        SERVER_DEMAND_ACTIVE_PDU.clone(),
        ShareControlHeader::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_demand_active() {
    let buf = CLIENT_DEMAND_ACTIVE_PDU_BUFFER.clone();

    assert_eq!(
        CLIENT_DEMAND_ACTIVE_PDU.clone(),
        ShareControlHeader::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_synchronize() {
    let buf = CLIENT_SYNCHRONIZE_BUFFER.as_ref();

    assert_eq!(
        CLIENT_SYNCHRONIZE.clone(),
        ShareControlHeader::from_buffer(buf).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_control_cooperate() {
    let buf = CONTROL_COOPERATE_BUFFER.as_ref();

    assert_eq!(
        CONTROL_COOPERATE.clone(),
        ShareControlHeader::from_buffer(buf).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_control_request_control() {
    let buf = CONTROL_REQUEST_CONTROL_BUFFER.as_ref();

    assert_eq!(
        CONTROL_REQUEST_CONTROL.clone(),
        ShareControlHeader::from_buffer(buf).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_server_control_granted_control() {
    let buf = SERVER_GRANTED_CONTROL_BUFFER.as_ref();

    assert_eq!(
        SERVER_GRANTED_CONTROL.clone(),
        ShareControlHeader::from_buffer(buf).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_font_list() {
    let buf = CLIENT_FONT_LIST_BUFFER.as_ref();

    assert_eq!(
        CLIENT_FONT_LIST.clone(),
        ShareControlHeader::from_buffer(buf).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_server_font_map() {
    let buf = SERVER_FONT_MAP_BUFFER.as_ref();

    assert_eq!(
        SERVER_FONT_MAP.clone(),
        ShareControlHeader::from_buffer(buf).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_server_monitor_layout() {
    let buf = MONITOR_LAYOUT_PDU_BUFFER.clone();

    assert_eq!(
        MONITOR_LAYOUT_PDU.clone(),
        ShareControlHeader::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_info() {
    let pdu = CLIENT_INFO_PDU.clone();
    let expected_buf = CLIENT_INFO_PDU_BUFFER.clone();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_license() {
    let pdu = CLIENT_LICENSE_PDU.clone();
    let expected_buf = CLIENT_LICENSE_PDU_BUFFER.clone();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_server_demand_active() {
    let pdu = SERVER_DEMAND_ACTIVE_PDU.clone();
    let expected_buf = SERVER_DEMAND_ACTIVE_PDU_BUFFER.clone();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_demand_active() {
    let pdu = CLIENT_DEMAND_ACTIVE_PDU.clone();
    let expected_buf = CLIENT_DEMAND_ACTIVE_PDU_BUFFER.clone();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_synchronize() {
    let pdu = CLIENT_SYNCHRONIZE.clone();
    let expected_buf = CLIENT_SYNCHRONIZE_BUFFER.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_control_cooperate() {
    let pdu = CONTROL_COOPERATE.clone();
    let expected_buf = CONTROL_COOPERATE_BUFFER.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_control_request_control() {
    let pdu = CONTROL_REQUEST_CONTROL.clone();
    let expected_buf = CONTROL_REQUEST_CONTROL_BUFFER.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_server_control_granted_control() {
    let pdu = SERVER_GRANTED_CONTROL.clone();
    let expected_buf = SERVER_GRANTED_CONTROL_BUFFER.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_font_list() {
    let pdu = CLIENT_FONT_LIST.clone();
    let expected_buf = CLIENT_FONT_LIST_BUFFER.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_server_font_map() {
    let pdu = SERVER_FONT_MAP.clone();
    let expected_buf = SERVER_FONT_MAP_BUFFER.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_server_monitor_layout() {
    let pdu = MONITOR_LAYOUT_PDU.clone();
    let expected_buf = MONITOR_LAYOUT_PDU_BUFFER.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_info() {
    let pdu = CLIENT_INFO_PDU.clone();
    let expected_buf_len = CLIENT_INFO_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_license() {
    let pdu = CLIENT_LICENSE_PDU.clone();
    let expected_buf_len = CLIENT_LICENSE_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_demand_active() {
    let pdu = SERVER_DEMAND_ACTIVE_PDU.clone();
    let expected_buf_len = SERVER_DEMAND_ACTIVE_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_demand_active() {
    let pdu = CLIENT_DEMAND_ACTIVE_PDU.clone();
    let expected_buf_len = CLIENT_DEMAND_ACTIVE_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_synchronize() {
    let pdu = CLIENT_SYNCHRONIZE.clone();
    let expected_buf_len = CLIENT_SYNCHRONIZE_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_control_cooperate() {
    let pdu = CONTROL_COOPERATE.clone();
    let expected_buf_len = CONTROL_COOPERATE_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_control_request_control() {
    let pdu = CONTROL_REQUEST_CONTROL.clone();
    let expected_buf_len = CONTROL_REQUEST_CONTROL_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_control_granted_control() {
    let pdu = SERVER_GRANTED_CONTROL.clone();
    let expected_buf_len = SERVER_GRANTED_CONTROL_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_font_list() {
    let pdu = CLIENT_FONT_LIST.clone();
    let expected_buf_len = CLIENT_FONT_LIST_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_font_map() {
    let pdu = SERVER_FONT_MAP.clone();
    let expected_buf_len = SERVER_FONT_MAP_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_monitor_layout() {
    let pdu = MONITOR_LAYOUT_PDU.clone();
    let expected_buf_len = MONITOR_LAYOUT_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}
