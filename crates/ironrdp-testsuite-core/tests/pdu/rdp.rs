use ironrdp_core::{Encode as _, decode, encode_vec};
use ironrdp_pdu::rdp::client_info::ClientInfo;
use ironrdp_testsuite_core::capsets::*;
use ironrdp_testsuite_core::client_info::*;
use ironrdp_testsuite_core::rdp::*;

#[test]
fn from_buffer_correctly_parses_rdp_pdu_client_info() {
    let buf = CLIENT_INFO_PDU_BUFFER;

    assert_eq!(CLIENT_INFO_PDU.clone(), decode(buf.as_slice()).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_server_license() {
    assert_eq!(*SERVER_LICENSE_PDU, decode(&SERVER_LICENSE_BUFFER).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_server_demand_active() {
    let buf = SERVER_DEMAND_ACTIVE_PDU_BUFFER;

    assert_eq!(SERVER_DEMAND_ACTIVE_PDU.clone(), decode(buf.as_slice()).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_client_demand_active() {
    let buf = CLIENT_DEMAND_ACTIVE_PDU_BUFFER;

    assert_eq!(CLIENT_DEMAND_ACTIVE_PDU.clone(), decode(buf.as_slice()).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_client_synchronize() {
    let buf = CLIENT_SYNCHRONIZE_BUFFER.as_ref();

    assert_eq!(CLIENT_SYNCHRONIZE.clone(), decode(buf).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_client_control_cooperate() {
    let buf = CONTROL_COOPERATE_BUFFER.as_ref();

    assert_eq!(CONTROL_COOPERATE.clone(), decode(buf).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_client_control_request_control() {
    let buf = CONTROL_REQUEST_CONTROL_BUFFER.as_ref();

    assert_eq!(CONTROL_REQUEST_CONTROL.clone(), decode(buf).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_server_control_granted_control() {
    let buf = SERVER_GRANTED_CONTROL_BUFFER.as_ref();

    assert_eq!(SERVER_GRANTED_CONTROL.clone(), decode(buf).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_client_font_list() {
    let buf = CLIENT_FONT_LIST_BUFFER.as_ref();

    assert_eq!(CLIENT_FONT_LIST.clone(), decode(buf).unwrap());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_server_font_map() {
    let buf = SERVER_FONT_MAP_BUFFER.as_ref();

    assert_eq!(SERVER_FONT_MAP.clone(), decode(buf).unwrap());
}

#[test]
fn from_header_only_buffer_defaults_rdp_pdu_server_font_map() {
    let buf = &SERVER_FONT_MAP_BUFFER[..18];

    assert_eq!(SERVER_FONT_MAP.clone(), decode(buf).unwrap());
}

/// VirtualBox's VRDP declares `totalLength` as the size of the two headers (18) and does not
/// count the 8-byte Font Map body that follows it, so the PDU is complete but under-declared.
#[test]
fn from_buffer_with_under_declared_total_length_parses_rdp_pdu_server_font_map() {
    let mut buf = SERVER_FONT_MAP_BUFFER;
    buf[0] = 18;

    assert_eq!(SERVER_FONT_MAP.clone(), decode(buf.as_ref()).unwrap());
}

/// A header-only Server Font Map that declares its own length honestly: 18 bytes sent, 18
/// declared. The decoder supports this compatibility encoding by substituting default Font Map
/// fields when the body is absent.
///
/// It is pinned separately because `ShareDataPdu::from_type` defaults a `FontPdu` in when the
/// cursor is empty, so the re-encoded size is 26 and comparing 18 against it rejected the PDU.
/// Anyone reinstating a comparison against the re-encoded size will fail here. The neighbouring
/// `from_header_only_buffer_defaults` test does not cover it — its fixture still declares 26.
#[test]
fn from_header_only_buffer_with_matching_total_length_parses_rdp_pdu_server_font_map() {
    let mut buf = SERVER_FONT_MAP_BUFFER[..18].to_vec();
    buf[0] = 18;

    assert_eq!(SERVER_FONT_MAP.clone(), decode(buf.as_slice()).unwrap());
}

#[test]
fn from_buffer_rejects_other_under_declared_total_lengths_for_server_font_map() {
    for total_length in (0u16..26).filter(|length| *length != 18) {
        let mut buf = SERVER_FONT_MAP_BUFFER;
        buf[..2].copy_from_slice(&total_length.to_le_bytes());

        assert!(
            decode::<ironrdp_pdu::rdp::headers::ShareControlHeader>(buf.as_ref()).is_err(),
            "accepted Server Font Map with totalLength {total_length}"
        );
    }
}

#[test]
fn from_header_only_buffer_rejects_rdp_pdu_client_font_list() {
    assert!(decode::<ironrdp_pdu::rdp::headers::ShareControlHeader>(&CLIENT_FONT_LIST_BUFFER[..18]).is_err());
}

#[test]
fn from_buffer_correctly_parses_rdp_pdu_server_monitor_layout() {
    let buf = MONITOR_LAYOUT_PDU_BUFFER.clone();

    assert_eq!(MONITOR_LAYOUT_PDU.clone(), decode(buf.as_slice()).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_client_info() {
    let buf = encode_vec(&*CLIENT_INFO_PDU).unwrap();
    assert_eq!(buf, CLIENT_INFO_PDU_BUFFER.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_server_license() {
    let buf = encode_vec(&*SERVER_LICENSE_PDU).unwrap();

    assert_eq!(SERVER_LICENSE_BUFFER.as_ref(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_server_demand_active() {
    let buf = encode_vec(&*SERVER_DEMAND_ACTIVE_PDU).unwrap();
    assert_eq!(buf, SERVER_DEMAND_ACTIVE_PDU_BUFFER.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_client_demand_active() {
    let buf = encode_vec(&*CLIENT_DEMAND_ACTIVE_PDU).unwrap();
    assert_eq!(buf, CLIENT_DEMAND_ACTIVE_PDU_BUFFER.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_client_synchronize() {
    let pdu = CLIENT_SYNCHRONIZE.clone();
    let expected_buf = CLIENT_SYNCHRONIZE_BUFFER.to_vec();

    let buf = encode_vec(&pdu).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_client_control_cooperate() {
    let pdu = CONTROL_COOPERATE.clone();
    let expected_buf = CONTROL_COOPERATE_BUFFER.to_vec();

    let buf = encode_vec(&pdu).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_client_control_request_control() {
    let pdu = CONTROL_REQUEST_CONTROL.clone();
    let expected_buf = CONTROL_REQUEST_CONTROL_BUFFER.to_vec();

    let buf = encode_vec(&pdu).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_server_control_granted_control() {
    let pdu = SERVER_GRANTED_CONTROL.clone();
    let expected_buf = SERVER_GRANTED_CONTROL_BUFFER.to_vec();

    let buf = encode_vec(&pdu).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_client_font_list() {
    let pdu = CLIENT_FONT_LIST.clone();
    let expected_buf = CLIENT_FONT_LIST_BUFFER.to_vec();

    let buf = encode_vec(&pdu).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_server_font_map() {
    let pdu = SERVER_FONT_MAP.clone();
    let expected_buf = SERVER_FONT_MAP_BUFFER.to_vec();

    let buf = encode_vec(&pdu).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correctly_serializes_rdp_pdu_server_monitor_layout() {
    let pdu = MONITOR_LAYOUT_PDU.clone();
    let expected_buf = MONITOR_LAYOUT_PDU_BUFFER.to_vec();

    let buf = encode_vec(&pdu).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_info() {
    let pdu = CLIENT_INFO_PDU.clone();
    let expected_buf_len = CLIENT_INFO_PDU_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_license() {
    let len = SERVER_LICENSE_PDU.size();

    assert_eq!(SERVER_LICENSE_BUFFER.len(), len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_demand_active() {
    let pdu = SERVER_DEMAND_ACTIVE_PDU.clone();
    let expected_buf_len = SERVER_DEMAND_ACTIVE_PDU_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_demand_active() {
    let pdu = CLIENT_DEMAND_ACTIVE_PDU.clone();
    let expected_buf_len = CLIENT_DEMAND_ACTIVE_PDU_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_synchronize() {
    let pdu = CLIENT_SYNCHRONIZE.clone();
    let expected_buf_len = CLIENT_SYNCHRONIZE_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_control_cooperate() {
    let pdu = CONTROL_COOPERATE.clone();
    let expected_buf_len = CONTROL_COOPERATE_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_control_request_control() {
    let pdu = CONTROL_REQUEST_CONTROL.clone();
    let expected_buf_len = CONTROL_REQUEST_CONTROL_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_control_granted_control() {
    let pdu = SERVER_GRANTED_CONTROL.clone();
    let expected_buf_len = SERVER_GRANTED_CONTROL_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_font_list() {
    let pdu = CLIENT_FONT_LIST.clone();
    let expected_buf_len = CLIENT_FONT_LIST_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_font_map() {
    let pdu = SERVER_FONT_MAP.clone();
    let expected_buf_len = SERVER_FONT_MAP_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_server_monitor_layout() {
    let pdu = MONITOR_LAYOUT_PDU.clone();
    let expected_buf_len = MONITOR_LAYOUT_PDU_BUFFER.len();

    let len = pdu.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_client_info_pdu_ansi() {
    assert_eq!(
        CLIENT_INFO_ANSI.clone(),
        decode(CLIENT_INFO_BUFFER_ANSI.as_ref()).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_client_info_pdu_unicode() {
    assert_eq!(
        CLIENT_INFO_UNICODE.clone(),
        decode(CLIENT_INFO_BUFFER_UNICODE.as_ref()).unwrap()
    );
}

#[test]
fn client_info_with_undefined_flag_bits_decodes_and_retains_them() {
    // [MS-RDPBCGR] 2.2.1.11.1.1's INFO_* list keeps growing; a client setting
    // a bit this library does not know yet must not be refused at the login
    // step (3.3.5.3.11 mandates no validation of this field). The unknown bit
    // is retained rather than dropped so re-encoding preserves the wire value.
    const UNDEFINED_BIT: u32 = 0x0000_0004; // undefined in 2.2.1.11.1.1

    let mut buffer = CLIENT_INFO_BUFFER_UNICODE.to_vec();
    let flags = u32::from_le_bytes(buffer[4..8].try_into().unwrap()) | UNDEFINED_BIT;
    buffer[4..8].copy_from_slice(&flags.to_le_bytes());

    let client_info: ClientInfo = decode(buffer.as_slice()).expect("undefined INFO bits are not fatal");

    assert_ne!(client_info.flags.bits() & UNDEFINED_BIT, 0);

    let reencoded = encode_vec(&client_info).unwrap();
    assert_eq!(reencoded, buffer);
}

#[test]
fn from_buffer_correct_parses_client_info_pdu_unicode_without_optional_fields() {
    assert_eq!(
        CLIENT_INFO_UNICODE_WITHOUT_OPTIONAL_FIELDS.clone(),
        decode(CLIENT_INFO_BUFFER_UNICODE_WITHOUT_OPTIONAL_FIELDS.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correct_serializes_client_info_pdu_ansi() {
    let data = CLIENT_INFO_ANSI.clone();
    let expected_buffer = CLIENT_INFO_BUFFER_ANSI.to_vec();

    let buffer = encode_vec(&data).unwrap();

    assert_eq!(expected_buffer, buffer);
}

#[test]
fn buffer_length_is_correct_for_client_info_pdu_ansi() {
    let data = CLIENT_INFO_ANSI.clone();
    let expected_buffer_len = CLIENT_INFO_BUFFER_ANSI.len();

    let len = data.size();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn to_buffer_correct_serializes_client_info_pdu_unicode() {
    let data = CLIENT_INFO_UNICODE.clone();
    let expected_buffer = CLIENT_INFO_BUFFER_UNICODE.to_vec();

    let buffer = encode_vec(&data).unwrap();

    assert_eq!(expected_buffer, buffer);
}

#[test]
fn buffer_length_is_correct_for_client_info_pdu_unicode() {
    let data = CLIENT_INFO_UNICODE.clone();
    let expected_buffer_len = CLIENT_INFO_BUFFER_UNICODE.len();

    let len = data.size();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn to_buffer_correct_serializes_client_info_pdu_unicode_without_optional_fields() {
    let data = CLIENT_INFO_UNICODE_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = CLIENT_INFO_BUFFER_UNICODE_WITHOUT_OPTIONAL_FIELDS.to_vec();

    let buffer = encode_vec(&data).unwrap();

    assert_eq!(expected_buffer, buffer);
}

#[test]
fn buffer_length_is_correct_for_client_info_pdu_unicode_without_optional_fields() {
    let data = CLIENT_INFO_UNICODE_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = CLIENT_INFO_BUFFER_UNICODE_WITHOUT_OPTIONAL_FIELDS.len();

    let len = data.size();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_demand_active() {
    let buffer = SERVER_DEMAND_ACTIVE_BUFFER.as_ref();

    assert_eq!(*SERVER_DEMAND_ACTIVE, decode(buffer).unwrap());
}

#[test]
fn from_buffer_correctly_parses_client_demand_active_with_incomplete_capability_set() {
    let buffer = CLIENT_DEMAND_ACTIVE_WITH_INCOMPLETE_CAPABILITY_SET_BUFFER.as_ref();

    assert_eq!(
        *CLIENT_DEMAND_ACTIVE_WITH_INCOMPLETE_CAPABILITY_SET,
        decode(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_demand_active() {
    let buffer = CLIENT_DEMAND_ACTIVE_BUFFER.as_ref();

    assert_eq!(*CLIENT_DEMAND_ACTIVE, decode(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_server_demand_active() {
    let data = SERVER_DEMAND_ACTIVE.clone();
    let expected_buffer = SERVER_DEMAND_ACTIVE_BUFFER.to_vec();

    let buff = encode_vec(&data).unwrap();

    assert_eq!(expected_buffer, buff);
}

#[test]
fn to_buffer_correctly_serializes_client_demand_active_with_incomplete_capability_set() {
    let data = CLIENT_DEMAND_ACTIVE_WITH_INCOMPLETE_CAPABILITY_SET.clone();
    let expected_buffer = CLIENT_DEMAND_ACTIVE_WITH_INCOMPLETE_CAPABILITY_SET_BUFFER.to_vec();

    let buff = encode_vec(&data).unwrap();

    assert_eq!(expected_buffer, buff);
}

#[test]
fn to_buffer_correctly_serializes_client_demand_active() {
    let data = CLIENT_DEMAND_ACTIVE.clone();
    let expected_buffer = CLIENT_DEMAND_ACTIVE_BUFFER.to_vec();

    let buff = encode_vec(&data).unwrap();

    assert_eq!(expected_buffer, buff);
}

#[test]
fn buffer_length_is_correct_for_server_demand_active() {
    let data = SERVER_DEMAND_ACTIVE.clone();
    let expected_buffer_len = SERVER_DEMAND_ACTIVE_BUFFER.len();

    let len = data.size();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_demand_active_with_incomplete_capability_set() {
    let data = CLIENT_DEMAND_ACTIVE_WITH_INCOMPLETE_CAPABILITY_SET.clone();
    let expected_buffer_len = CLIENT_DEMAND_ACTIVE_WITH_INCOMPLETE_CAPABILITY_SET_BUFFER.len();

    let len = data.size();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_demand_active() {
    let data = CLIENT_DEMAND_ACTIVE.clone();
    let expected_buffer_len = CLIENT_DEMAND_ACTIVE_BUFFER.len();

    let len = data.size();

    assert_eq!(expected_buffer_len, len);
}

/// Regression for issue #1292: decoding a `BitmapCacheV3` capability set then re-encoding it
/// must not reach `unreachable!()` in the `Encode` impl. The decoder accepts
/// `CapabilitySetType::BitmapCacheV3CodecID` (0x06) and stores the body in
/// `CapabilitySet::BitmapCacheV3(Vec<u8>)`; before the fix the inner `match` of the encoder's
/// catch-all arm did not cover that variant.
#[test]
fn bitmap_cache_v3_round_trip_does_not_panic() {
    use ironrdp_pdu::rdp::capability_sets::CapabilitySet;

    // 4-byte capability-set header with type=BitmapCacheV3CodecID(0x06) and length=4 (header only)
    let input: [u8; 4] = [0x06, 0x00, 0x04, 0x00];

    let decoded: CapabilitySet = decode(&input).expect("decode BitmapCacheV3 capability set");
    assert!(matches!(decoded, CapabilitySet::BitmapCacheV3(_)));

    let encoded = encode_vec(&decoded).expect("re-encode must not panic");
    assert_eq!(encoded, input, "round-trip must reproduce the original bytes");
}
