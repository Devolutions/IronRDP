use ironrdp_pdu::gcc::*;

use crate::gcc;

pub const CONFERENCE_CREATE_REQUEST_PREFIX_BUFFER: [u8; 23] = [
    0x00, 0x05, 0x00, 0x14, 0x7c, 0x00, 0x01, 0x81, 0x28, 0x00, 0x08, 0x00, 0x10, 0x00, 0x01, 0xc0, 0x00, 0x44, 0x75,
    0x63, 0x61, 0x81, 0x1c,
];

pub const CONFERENCE_CREATE_RESPONSE_PREFIX_BUFFER: [u8; 24] = [
    0x00, 0x05, 0x00, 0x14, 0x7c, 0x00, 0x01, 0x81, 0x15, 0x14, 0x76, 0x0a, 0x01, 0x01, 0x00, 0x01, 0xc0, 0x00, 0x4d,
    0x63, 0x44, 0x6e, 0x81, 0x08,
];

lazy_static! {
    pub static ref CONFERENCE_CREATE_REQUEST: ConferenceCreateRequest = ConferenceCreateRequest {
        gcc_blocks: gcc::CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone(),
    };
    pub static ref CONFERENCE_CREATE_RESPONSE: ConferenceCreateResponse = ConferenceCreateResponse {
        user_id: 0x79f3,
        gcc_blocks: gcc::SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone(),
    };
}

pub const CONFERENCE_CREATE_REQUEST_BUFFER: [u8; concat_arrays_size!(
    CONFERENCE_CREATE_REQUEST_PREFIX_BUFFER,
    gcc::CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER
)] = concat_arrays!(
    CONFERENCE_CREATE_REQUEST_PREFIX_BUFFER,
    gcc::CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER
);

pub const CONFERENCE_CREATE_RESPONSE_BUFFER: [u8; concat_arrays_size!(
    CONFERENCE_CREATE_RESPONSE_PREFIX_BUFFER,
    gcc::SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER
)] = concat_arrays!(
    CONFERENCE_CREATE_RESPONSE_PREFIX_BUFFER,
    gcc::SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER
);
