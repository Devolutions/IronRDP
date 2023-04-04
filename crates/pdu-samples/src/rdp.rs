use crate::capsets::{
    CLIENT_DEMAND_ACTIVE, CLIENT_DEMAND_ACTIVE_BUFFER, SERVER_DEMAND_ACTIVE, SERVER_DEMAND_ACTIVE_BUFFER,
};
use crate::client_info::{CLIENT_INFO_BUFFER_UNICODE, CLIENT_INFO_UNICODE};
use crate::monitor_data::MONITOR_DATA_WITH_MONITORS_BUFFER;
use ironrdp_pdu::gcc;
use ironrdp_pdu::rdp::finalization_messages::*;
use ironrdp_pdu::rdp::headers::*;
use ironrdp_pdu::rdp::server_license::*;
use ironrdp_pdu::rdp::*;

pub const CLIENT_INFO_PDU_SECURITY_HEADER_BUFFER: [u8; 4] = [
    0x40, 0x00, // flags
    0x00, 0x00, // flagsHi
];

pub const SERVER_DEMAND_ACTIVE_PDU_HEADERS_BUFFER: [u8; 10] = [
    0x6f, 0x01, // ShareControlHeader::totalLength
    0x11, 0x00, // ShareControlHeader::pduType
    0xea, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
];

pub const CLIENT_DEMAND_ACTIVE_PDU_HEADERS_BUFFER: [u8; 10] = [
    0xf0, 0x01, // ShareControlHeader::totalLength
    0x13, 0x00, // ShareControlHeader::pduType
    0xef, 0x03, // ShareControlHeader::PduSource
    0xea, 0x03, 0x01, 0x00, // share id
];

pub const MONITOR_LAYOUT_HEADERS_BUFFER: [u8; 18] = [
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

pub const CLIENT_SYNCHRONIZE_BUFFER: [u8; 22] = [
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

pub const CONTROL_COOPERATE_BUFFER: [u8; 26] = [
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

pub const CONTROL_REQUEST_CONTROL_BUFFER: [u8; 26] = [
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

pub const SERVER_GRANTED_CONTROL_BUFFER: [u8; 26] = [
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

pub const CLIENT_FONT_LIST_BUFFER: [u8; 26] = [
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

pub const SERVER_FONT_MAP_BUFFER: [u8; 26] = [
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

pub const SERVER_LICENSE_BUFFER: [u8; 20] = [
    0x80, 0x00, // flags
    0x00, 0x00, // flagsHi
    0xff, 0x03, 0x10, 0x00, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
];

lazy_static! {
    pub static ref CLIENT_INFO_PDU: ClientInfoPdu = ClientInfoPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::INFO_PKT,
        },
        client_info: CLIENT_INFO_UNICODE.clone(),
    };
    pub static ref SERVER_LICENSE_PDU: InitialServerLicenseMessage = InitialServerLicenseMessage {
        license_header: LicenseHeader {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::LICENSE_PKT,
            },
            preamble_message_type: PreambleType::ErrorAlert,
            preamble_flags: PreambleFlags::empty(),
            preamble_version: PreambleVersion::V3,
            preamble_message_size: (SERVER_LICENSE_BUFFER.len() - BASIC_SECURITY_HEADER_SIZE) as u16
        },
        message_type: InitialMessageType::StatusValidClient(LicensingErrorMessage {
            error_code: LicenseErrorCode::StatusValidClient,
            state_transition: LicensingStateTransition::NoTransition,
            error_info: Vec::new(),
        })
    };
    pub static ref SERVER_DEMAND_ACTIVE_PDU: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()),
        pdu_source: 1002,
        share_id: 66_538,
    };
    pub static ref CLIENT_DEMAND_ACTIVE_PDU: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::ClientConfirmActive(CLIENT_DEMAND_ACTIVE.clone()),
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref CLIENT_SYNCHRONIZE: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::Synchronize(SynchronizePdu { target_user_id: 0x03ea }),
            stream_priority: StreamPriority::Low,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
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
        pdu_source: 1002,
        share_id: 66_538,
    };
    pub static ref MONITOR_LAYOUT_PDU: ShareControlHeader = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::MonitorLayout(MonitorLayoutPdu {
                monitors: crate::monitor_data::MONITOR_DATA_WITH_MONITORS.monitors.clone(),
            }),
            stream_priority: StreamPriority::Low,
            compression_flags: CompressionFlags::empty(),
            compression_type: client_info::CompressionType::K8,
        }),
        pdu_source: 1007,
        share_id: 66_538,
    };
    pub static ref MONITOR_LAYOUT_PDU_BUFFER: Vec<u8> = {
        let mut buffer = MONITOR_LAYOUT_HEADERS_BUFFER.to_vec();
        buffer.extend(
            MONITOR_DATA_WITH_MONITORS_BUFFER
                .to_vec()
                .split_off(gcc::MONITOR_FLAGS_SIZE),
        );
        buffer
    };
}

pub const CLIENT_INFO_PDU_BUFFER: [u8; concat_arrays_size!(
    CLIENT_INFO_PDU_SECURITY_HEADER_BUFFER,
    CLIENT_INFO_BUFFER_UNICODE
)] = concat_arrays!(CLIENT_INFO_PDU_SECURITY_HEADER_BUFFER, CLIENT_INFO_BUFFER_UNICODE);

pub const SERVER_DEMAND_ACTIVE_PDU_BUFFER: [u8; concat_arrays_size!(
    SERVER_DEMAND_ACTIVE_PDU_HEADERS_BUFFER,
    SERVER_DEMAND_ACTIVE_BUFFER
)] = concat_arrays!(SERVER_DEMAND_ACTIVE_PDU_HEADERS_BUFFER, SERVER_DEMAND_ACTIVE_BUFFER);

pub const CLIENT_DEMAND_ACTIVE_PDU_BUFFER: [u8; concat_arrays_size!(
    CLIENT_DEMAND_ACTIVE_PDU_HEADERS_BUFFER,
    CLIENT_DEMAND_ACTIVE_BUFFER
)] = concat_arrays!(CLIENT_DEMAND_ACTIVE_PDU_HEADERS_BUFFER, CLIENT_DEMAND_ACTIVE_BUFFER);
