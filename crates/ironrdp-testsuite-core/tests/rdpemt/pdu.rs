use ironrdp_core::DecodeResult;
use ironrdp_rdpemt::pdu::*;
#[test]
fn dispatch_create_request() {
    let wire: &[u8] = &[
        0x00, 0x18, 0x00, 0x04, // Header
        0x01, 0x00, 0x00, 0x00, // RequestID = 1
        0x00, 0x00, 0x00, 0x00, // Reserved
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Cookie (16 bytes)
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    let pdu: TunnelPdu = ironrdp_core::decode(wire).expect("decode");
    match pdu {
        TunnelPdu::CreateRequest(req) => {
            assert_eq!(req.request_id, 1);
        }
        other => panic!("expected CreateRequest, got {other:?}"),
    }
}

#[test]
fn dispatch_create_response() {
    let wire: &[u8] = &[
        0x01, 0x04, 0x00, 0x04, // Header
        0x00, 0x00, 0x00, 0x00, // HrResponse = S_OK
    ];

    let pdu: TunnelPdu = ironrdp_core::decode(wire).expect("decode");
    match pdu {
        TunnelPdu::CreateResponse(resp) => {
            assert!(resp.is_success());
        }
        other => panic!("expected CreateResponse, got {other:?}"),
    }
}

#[test]
fn dispatch_data() {
    let wire: &[u8] = &[
        0x02, 0x03, 0x00, 0x04, // Header: Data, payload=3, header=4
        0xAA, 0xBB, 0xCC, // HigherLayerData
    ];

    let pdu: TunnelPdu = ironrdp_core::decode(wire).expect("decode");
    match pdu {
        TunnelPdu::Data(data) => {
            assert_eq!(data.higher_layer_data, [0xAA, 0xBB, 0xCC]);
            assert!(data.sub_headers.is_empty());
        }
        other => panic!("expected Data, got {other:?}"),
    }
}

#[test]
fn dispatch_unknown_action() {
    let wire: &[u8] = &[0x0F, 0x00, 0x00, 0x04];
    let result: DecodeResult<TunnelPdu> = ironrdp_core::decode(wire);
    assert!(result.is_err());
}
