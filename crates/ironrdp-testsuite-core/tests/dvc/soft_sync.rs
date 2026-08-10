use super::*;

const CHANNEL_ID: u32 = 0x0303;
const REQUEST_ENCODED: [u8; 20] = [
    0x80, 0x00, 0x12, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x03, 0x00,
    0x00,
];
const RESPONSE_ENCODED: [u8; 10] = [0x90, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];

fn request() -> SoftSyncRequestPdu {
    SoftSyncRequestPdu::new(vec![SoftSyncChannelList::new(
        SoftSyncTunnelType::RELIABLE_UDP,
        vec![CHANNEL_ID],
    )])
}

fn response() -> SoftSyncResponsePdu {
    SoftSyncResponsePdu::new(vec![SoftSyncTunnelType::RELIABLE_UDP])
}

#[test]
fn encodes_soft_sync_request() {
    test_encodes(&DrdynvcServerPdu::SoftSyncRequest(request()), &REQUEST_ENCODED);
}

#[test]
fn decodes_soft_sync_request() {
    test_decodes(&REQUEST_ENCODED, &DrdynvcServerPdu::SoftSyncRequest(request()));
}

#[test]
fn encodes_soft_sync_response() {
    test_encodes(&DrdynvcClientPdu::SoftSyncResponse(response()), &RESPONSE_ENCODED);
}

#[test]
fn decodes_soft_sync_response() {
    test_decodes(&RESPONSE_ENCODED, &DrdynvcClientPdu::SoftSyncResponse(response()));
}

#[test]
fn rejects_soft_sync_request_with_mismatched_length() {
    let mut request = REQUEST_ENCODED;
    request[2] = 0x11;

    let mut src = ReadCursor::new(&request);
    assert!(DrdynvcServerPdu::decode(&mut src).is_err());
}

#[test]
fn rejects_soft_sync_request_with_duplicate_channel_id() {
    let request = [
        0x80, 0x00, 0x1C, 0x00, 0x00, 0x00, 0x03, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x03,
        0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x03, 0x00, 0x00,
    ];

    let mut src = ReadCursor::new(&request);
    assert!(DrdynvcServerPdu::decode(&mut src).is_err());
}

#[test]
fn rejects_soft_sync_response_with_duplicate_tunnel_type() {
    let response = [
        0x90, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];

    let mut src = ReadCursor::new(&response);
    assert!(DrdynvcClientPdu::decode(&mut src).is_err());
}

#[test]
fn rejects_soft_sync_response_with_too_many_tunnels() {
    let response = [0x90, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];

    let mut src = ReadCursor::new(&response);
    assert!(DrdynvcClientPdu::decode(&mut src).is_err());
}

#[test]
fn rejects_soft_sync_request_with_too_many_tunnels() {
    let request = [0x80, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0xFF, 0xFF];

    let mut src = ReadCursor::new(&request);
    assert!(DrdynvcServerPdu::decode(&mut src).is_err());
}

#[test]
fn decodes_unknown_soft_sync_tunnel_type() {
    let response = [0x90, 0x00, 0x01, 0x00, 0x00, 0x00, 0x7F, 0x00, 0x00, 0x00];

    let mut src = ReadCursor::new(&response);
    let decoded = DrdynvcClientPdu::decode(&mut src).unwrap();
    let DrdynvcClientPdu::SoftSyncResponse(response) = decoded else {
        panic!("expected Soft-Sync response");
    };

    assert_eq!(response.tunnels_to_switch(), &[SoftSyncTunnelType::from(0x7F)]);
}
