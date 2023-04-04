use std::borrow::Cow;

use ironrdp_pdu::mcs::*;

use crate::{
    conference_create::{
        CONFERENCE_CREATE_REQUEST, CONFERENCE_CREATE_REQUEST_BUFFER, CONFERENCE_CREATE_RESPONSE,
        CONFERENCE_CREATE_RESPONSE_BUFFER,
    },
    rdp::{CLIENT_INFO_PDU_BUFFER, SERVER_LICENSE_BUFFER},
};

pub const ERECT_DOMAIN_PDU_BUFFER: [u8; 5] = [0x04, 0x01, 0x00, 0x01, 0x00];

pub const ERECT_DOMAIN_PDU: ErectDomainPdu = ErectDomainPdu {
    sub_height: 0,
    sub_interval: 0,
};

pub const ATTACH_USER_REQUEST_PDU_BUFFER: [u8; 1] = [0x28];

pub const ATTACH_USER_REQUEST_PDU: AttachUserRequest = AttachUserRequest;

pub const ATTACH_USER_CONFIRM_PDU_BUFFER: [u8; 4] = [0x2e, 0x00, 0x00, 0x06];

pub const ATTACH_USER_CONFIRM_PDU: AttachUserConfirm = AttachUserConfirm {
    result: 0,
    initiator_id: 1007,
};

pub const CHANNEL_JOIN_REQUEST_PDU_BUFFER: [u8; 5] = [0x38, 0x00, 0x06, 0x03, 0xef];

pub const CHANNEL_JOIN_REQUEST_PDU: ChannelJoinRequest = ChannelJoinRequest {
    initiator_id: 1007,
    channel_id: 1007,
};

pub const CHANNEL_JOIN_CONFIRM_PDU_BUFFER: [u8; 8] = [0x3e, 0x00, 0x00, 0x06, 0x03, 0xef, 0x03, 0xef];

pub const CHANNEL_JOIN_CONFIRM_PDU: ChannelJoinConfirm = ChannelJoinConfirm {
    result: 0,
    initiator_id: 1007,
    requested_channel_id: 1007,
    channel_id: 1007,
};

pub const DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER: [u8; 2] = [0x21, 0x80];

pub const DISCONNECT_PROVIDER_ULTIMATUM_PDU: DisconnectProviderUltimatum = DisconnectProviderUltimatum {
    reason: DisconnectReason::UserRequested,
};

pub const SEND_DATA_REQUEST_PDU_BUFFER_PREFIX: [u8; 8] = [0x64, 0x00, 0x06, 0x03, 0xeb, 0x70, 0x81, 0x92];

pub const SEND_DATA_REQUEST_PDU_BUFFER: [u8; concat_arrays_size!(
    SEND_DATA_REQUEST_PDU_BUFFER_PREFIX,
    CLIENT_INFO_PDU_BUFFER
)] = concat_arrays!(SEND_DATA_REQUEST_PDU_BUFFER_PREFIX, CLIENT_INFO_PDU_BUFFER);

pub const SEND_DATA_REQUEST_PDU: OwnedSendDataRequest = SendDataRequest {
    initiator_id: 1007,
    channel_id: 1003,
    user_data: Cow::Borrowed(&CLIENT_INFO_PDU_BUFFER),
};

pub const SEND_DATA_INDICATION_PDU_BUFFER_PREFIX: [u8; 7] = [0x68, 0x00, 0x01, 0x03, 0xeb, 0x70, 0x14];

pub const SEND_DATA_INDICATION_PDU_BUFFER: [u8; concat_arrays_size!(
    SEND_DATA_INDICATION_PDU_BUFFER_PREFIX,
    SERVER_LICENSE_BUFFER
)] = concat_arrays!(SEND_DATA_INDICATION_PDU_BUFFER_PREFIX, SERVER_LICENSE_BUFFER);

pub const SEND_DATA_INDICATION_PDU: OwnedSendDataIndication = SendDataIndication {
    initiator_id: 1002,
    channel_id: 1003,
    user_data: Cow::Borrowed(&SERVER_LICENSE_BUFFER),
};

pub const CONNECT_INITIAL_PREFIX_BUFFER: [u8; 107] = [
    0x7f, 0x65, 0x82, 0x01, 0x99, 0x04, 0x01, 0x01, 0x04, 0x01, 0x01, 0x01, 0x01, 0xff, 0x30, 0x1a, 0x02, 0x01, 0x22,
    0x02, 0x01, 0x02, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x03, 0x00, 0xff,
    0xff, 0x02, 0x01, 0x02, 0x30, 0x19, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01, 0x02,
    0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x02, 0x04, 0x20, 0x02, 0x01, 0x02, 0x30, 0x20, 0x02, 0x03, 0x00, 0xff, 0xff,
    0x02, 0x03, 0x00, 0xfc, 0x17, 0x02, 0x03, 0x00, 0xff, 0xff, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01,
    0x02, 0x03, 0x00, 0xff, 0xff, 0x02, 0x01, 0x02, 0x04, 0x82, 0x01, 0x33,
];

pub const CONNECT_RESPONSE_PREFIX_BUFFER: [u8; 43] = [
    0x7f, 0x66, 0x82, 0x01, 0x46, 0x0a, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x1a, 0x02, 0x01, 0x22, 0x02, 0x01, 0x03,
    0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x03, 0x00, 0xff, 0xf8, 0x02, 0x01,
    0x02, 0x04, 0x82, 0x01, 0x20,
];

pub const CONNECT_INITIAL_BUFFER: [u8; concat_arrays_size!(
    CONNECT_INITIAL_PREFIX_BUFFER,
    CONFERENCE_CREATE_REQUEST_BUFFER
)] = concat_arrays!(CONNECT_INITIAL_PREFIX_BUFFER, CONFERENCE_CREATE_REQUEST_BUFFER);

pub const CONNECT_RESPONSE_BUFFER: [u8; concat_arrays_size!(
    CONNECT_RESPONSE_PREFIX_BUFFER,
    CONFERENCE_CREATE_RESPONSE_BUFFER
)] = concat_arrays!(CONNECT_RESPONSE_PREFIX_BUFFER, CONFERENCE_CREATE_RESPONSE_BUFFER);

lazy_static! {
    pub static ref CONNECT_INITIAL: ConnectInitial = ConnectInitial {
        calling_domain_selector: vec![0x01],
        called_domain_selector: vec![0x01],
        upward_flag: true,
        target_parameters: DomainParameters {
            max_channel_ids: 34,
            max_user_ids: 2,
            max_token_ids: 0,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        },
        min_parameters: DomainParameters {
            max_channel_ids: 1,
            max_user_ids: 1,
            max_token_ids: 1,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 1056,
            protocol_version: 2,
        },
        max_parameters: DomainParameters {
            max_channel_ids: 65535,
            max_user_ids: 64535,
            max_token_ids: 65535,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        },
        conference_create_request: CONFERENCE_CREATE_REQUEST.clone(),
    };
    pub static ref CONNECT_RESPONSE: ConnectResponse = ConnectResponse {
        called_connect_id: 0,
        domain_parameters: DomainParameters {
            max_channel_ids: 34,
            max_user_ids: 3,
            max_token_ids: 0,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65528,
            protocol_version: 2,
        },
        conference_create_response: CONFERENCE_CREATE_RESPONSE.clone(),
    };
}
