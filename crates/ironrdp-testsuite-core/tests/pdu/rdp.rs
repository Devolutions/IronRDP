use ironrdp_core::{decode, encode_vec, Encode as _};
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
