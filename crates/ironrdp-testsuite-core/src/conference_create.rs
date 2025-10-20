use std::sync::LazyLock;

use array_concat::{concat_arrays, concat_arrays_size};
use ironrdp_pdu::gcc::{ConferenceCreateRequest, ConferenceCreateResponse};

use crate::gcc;

pub const CONFERENCE_CREATE_REQUEST_PREFIX_BUFFER: [u8; 23] = [
    0x00, 0x05, 0x00, 0x14, 0x7c, 0x00, 0x01, 0x81, 0x28, 0x00, 0x08, 0x00, 0x10, 0x00, 0x01, 0xc0, 0x00, 0x44, 0x75,
    0x63, 0x61, 0x81, 0x1c,
];

pub const CONFERENCE_CREATE_RESPONSE_PREFIX_BUFFER: [u8; 24] = [
    0x00, 0x05, 0x00, 0x14, 0x7c, 0x00, 0x01, 0x81, 0x16, 0x14, 0x76, 0x0a, 0x01, 0x01, 0x00, 0x01, 0xc0, 0x00, 0x4d,
    0x63, 0x44, 0x6e, 0x81, 0x08,
];

pub static CONFERENCE_CREATE_REQUEST: LazyLock<ConferenceCreateRequest> = LazyLock::new(|| {
    ConferenceCreateRequest::new(gcc::CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone()).expect("should not fail")
});
pub static CONFERENCE_CREATE_RESPONSE: LazyLock<ConferenceCreateResponse> = LazyLock::new(|| {
    ConferenceCreateResponse::new(0x79f3, gcc::SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone()).expect("should not fail")
});

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
