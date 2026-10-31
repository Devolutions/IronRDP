use ironrdp_pdu::{CREDSSP_TS_REQUEST_HINT, PduHint as _};
use rstest::rstest;

// A TSRequest is a DER-encoded SEQUENCE: tag 0x30, then the length octets.
// oracle: spec ITU-T X.690 §8.1.3
#[rstest]
#[case::empty(&[])]
#[case::tag_only(&[0x30])]
#[case::long_form_missing_a_length_octet(&[0x30, 0x82, 0x01])]
fn credssp_hint_waits_for_the_rest_of_a_split_header(#[case] bytes: &[u8]) {
    assert_eq!(CREDSSP_TS_REQUEST_HINT.find_size(bytes).unwrap(), None);
}

// The size is the tag octet, the length octets, and the content length they encode.
// oracle: spec ITU-T X.690 §8.1.3
#[rstest]
#[case::short_form(&[0x30, 0x7F], 1 + 1 + 0x7F)]
#[case::long_form_one_octet(&[0x30, 0x81, 0x80], 1 + 2 + 0x80)]
#[case::long_form_two_octets(&[0x30, 0x82, 0x01, 0x00], 1 + 3 + 0x100)]
fn credssp_hint_sizes_a_ts_request_from_its_header(#[case] bytes: &[u8], #[case] size: usize) {
    assert_eq!(CREDSSP_TS_REQUEST_HINT.find_size(bytes).unwrap(), Some((true, size)));
}

// No further bytes can turn these headers into a TSRequest that can be decoded, so the hint must reject them rather
// than wait.
#[rstest]
// 0x02 is the INTEGER tag, not the SEQUENCE tag.
// oracle: spec ITU-T X.690 §8.1.2
#[case::not_a_sequence(&[0x02])]
// 0x80 announces the indefinite length form, which DER forbids.
// oracle: spec ITU-T X.690 §10.1
#[case::indefinite_length(&[0x30, 0x80])]
// 0x83 announces three length octets, a TSRequest of 64 KiB or more, which sspi cannot decode.
// oracle: reference Devolutions/sspi-rs@b21f4581a880:src/ber/mod.rs (read_length accepts one or two length octets)
#[case::three_length_octets(&[0x30, 0x83, 0x01, 0x00, 0x00])]
fn credssp_hint_rejects_a_header_that_can_never_frame(#[case] bytes: &[u8]) {
    assert!(CREDSSP_TS_REQUEST_HINT.find_size(bytes).is_err());
}
