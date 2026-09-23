//! Decoding of client PDUs on the MCS message channel ([MS-RDPBCGR] 2.2.1.3.7).

use ironrdp_core::{decode, encode_vec};
use ironrdp_pdu::rdp::autodetect::{AutoDetectResponse, AutoDetectRspPdu};
use ironrdp_pdu::rdp::message_channel::ClientMessageChannelPdu;
use ironrdp_pdu::rdp::multitransport::MultitransportResponsePdu;

#[test]
fn autodetect_response_is_recognized() {
    let pdu = AutoDetectRspPdu::new(AutoDetectResponse::RttResponse { sequence_number: 7 });
    let bytes = encode_vec(&pdu).unwrap();

    let decoded = decode::<ClientMessageChannelPdu>(&bytes).unwrap();

    assert_eq!(decoded, ClientMessageChannelPdu::AutoDetectResponse(pdu));
}

/// A client that offered UDP multitransport but could not establish it answers
/// on the message channel with an Initiate Multitransport Response (MS-RDPBCGR
/// 2.2.15.2), not an auto-detect response.
#[test]
fn multitransport_response_is_recognized() {
    let pdu = MultitransportResponsePdu::abort(42);
    let bytes = encode_vec(&pdu).unwrap();

    let decoded = decode::<ClientMessageChannelPdu>(&bytes).unwrap();

    assert_eq!(decoded, ClientMessageChannelPdu::MultitransportResponse(pdu));
}

#[test]
fn both_variants_round_trip() {
    for pdu in [
        ClientMessageChannelPdu::AutoDetectResponse(AutoDetectRspPdu::new(AutoDetectResponse::RttResponse {
            sequence_number: 3,
        })),
        ClientMessageChannelPdu::MultitransportResponse(MultitransportResponsePdu::abort(9)),
    ] {
        let bytes = encode_vec(&pdu).unwrap();
        assert_eq!(decode::<ClientMessageChannelPdu>(&bytes).unwrap(), pdu);
    }
}

/// Dispatch follows the security header: a truncated PDU whose header says
/// SEC_TRANSPORT_RSP is reported as a malformed multitransport response, not
/// as a failed auto-detect decode.
#[test]
fn truncated_multitransport_response_is_reported_as_one() {
    let bytes = encode_vec(&MultitransportResponsePdu::abort(42)).unwrap();

    let error = decode::<ClientMessageChannelPdu>(&bytes[..8]).unwrap_err();

    let message = format!("{error:#}");
    assert!(message.contains("MultitransportResponsePdu"), "{message}");
}

/// A header without SEC_TRANSPORT_RSP goes to the auto-detect decoder, which
/// reports the flag it expected.
#[test]
fn unrecognized_payload_is_reported_by_the_autodetect_decoder() {
    let bytes = [0x00u8; 12];

    let error = decode::<ClientMessageChannelPdu>(&bytes).unwrap_err();

    let message = format!("{error:#}");
    assert!(message.contains("SEC_AUTODETECT_RSP"), "{message}");
}

#[test]
fn input_shorter_than_the_flags_field_is_rejected() {
    assert!(decode::<ClientMessageChannelPdu>(&[0x04]).is_err());
}
