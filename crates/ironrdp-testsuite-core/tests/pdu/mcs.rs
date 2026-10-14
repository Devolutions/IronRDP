use expect_test::expect;
use ironrdp_core::{Encode as _, decode, encode_vec};
use ironrdp_pdu::mcs::*;
use ironrdp_testsuite_core::mcs::*;
use ironrdp_testsuite_core::mcs_encode_decode_test;

fn mcs_decode<'de, T: McsPdu<'de>>(src: &'de [u8]) -> ironrdp_core::DecodeResult<T> {
    let mut cursor = ironrdp_core::ReadCursor::new(src);
    T::mcs_body_decode(&mut cursor, src.len())
}

#[test]
fn invalid_domain_mcspdu() {
    let e = mcs_decode::<McsMessage<'_>>(&[0x48, 0x00, 0x00, 0x00, 0x70, 0x00, 0x01, 0x03, 0xEB, 0x70, 0x14])
        .err()
        .unwrap();

    expect![[r#"
        Error {
            context: "McsMessage",
            kind: InvalidField {
                field: "domain-mcspdu",
                reason: "unexpected application tag for CHOICE",
                offset: Some(
                    0,
                ),
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&e);
}

mcs_encode_decode_test! {
    erect_domain_request: ERECT_DOMAIN_PDU, ERECT_DOMAIN_PDU_BUFFER;
    attach_user_request: ATTACH_USER_REQUEST_PDU, ATTACH_USER_REQUEST_PDU_BUFFER;
    attach_user_confirm: ATTACH_USER_CONFIRM_PDU, ATTACH_USER_CONFIRM_PDU_BUFFER;
    channel_join_request: CHANNEL_JOIN_REQUEST_PDU, CHANNEL_JOIN_REQUEST_PDU_BUFFER;
    channel_join_confirm: CHANNEL_JOIN_CONFIRM_PDU, CHANNEL_JOIN_CONFIRM_PDU_BUFFER;
    send_data_request: SEND_DATA_REQUEST_PDU, SEND_DATA_REQUEST_PDU_BUFFER;
    send_data_indication: SEND_DATA_INDICATION_PDU, SEND_DATA_INDICATION_PDU_BUFFER;
    disconnect_ultimatum: DISCONNECT_PROVIDER_ULTIMATUM_PDU, DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER;
}

#[test]
fn from_buffer_correct_parses_connect_initial() {
    let blocks: ConnectInitial = decode(CONNECT_INITIAL_BUFFER.as_slice()).unwrap();
    assert_eq!(blocks, *CONNECT_INITIAL);
}

#[test]
fn to_buffer_correct_serializes_connect_initial() {
    let buf = encode_vec(&*CONNECT_INITIAL).unwrap();
    assert_eq!(buf, CONNECT_INITIAL_BUFFER);
}

#[test]
fn buffer_length_is_correct_for_connect_initial() {
    let len = CONNECT_INITIAL.size();
    assert_eq!(len, CONNECT_INITIAL_BUFFER.len());
}

#[test]
fn from_buffer_correct_parses_connect_response() {
    let blocks: ConnectResponse = decode(CONNECT_RESPONSE_BUFFER.as_slice()).unwrap();
    assert_eq!(blocks, *CONNECT_RESPONSE);
}

#[test]
fn to_buffer_correct_serializes_connect_response() {
    let buf = encode_vec(&*CONNECT_RESPONSE).unwrap();
    assert_eq!(buf, CONNECT_RESPONSE_BUFFER);
}

#[test]
fn buffer_length_is_correct_for_connect_response() {
    let len = CONNECT_RESPONSE.size();
    assert_eq!(len, CONNECT_RESPONSE_BUFFER.len());
}

/// Decoding any prefix of a valid Connect Initial must never panic.
///
/// A Connect Initial truncated right before the PER padding following the conference name used to move the
/// read cursor past the end of the buffer.
///
/// Some prefixes legitimately decode (the outer BER and PER lengths are not enforced, and a cut at a GCC block
/// boundary yields a shorter, valid set of blocks), so only the absence of panics is asserted here.
#[test]
fn decoding_truncated_connect_initial_does_not_panic() {
    for len in 0..CONNECT_INITIAL_BUFFER.len() {
        let truncated = &CONNECT_INITIAL_BUFFER[..len];
        let result = std::panic::catch_unwind(|| decode::<ConnectInitial>(truncated));
        assert!(result.is_ok(), "prefix of length {len} panicked during decoding");
    }
}

#[test]
fn connect_initial_truncated_before_conference_name_padding() {
    // 107 bytes of MCS Connect Initial header, then the conference create request up to and including the
    // conference name: key choice (1), object identifier (6), connectPDU length (2), ConnectGCCPDU choice (1),
    // selection (1), conference name length (1) and conference name (1).
    const PADDING_OFFSET: usize = CONNECT_INITIAL_PREFIX_BUFFER.len() + 13;

    let e = decode::<ConnectInitial>(&CONNECT_INITIAL_BUFFER[..PADDING_OFFSET])
        .err()
        .unwrap();

    expect![[r#"
        Error {
            context: "<ironrdp_pdu::gcc::conference_create::ConferenceCreateRequest as ironrdp_core::decode::Decode<'_>>::decode",
            kind: NotEnoughBytes {
                received: 0,
                expected: 1,
                offset: Some(
                    120,
                ),
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&e);
}
