#![expect(
    unused_crate_dependencies,
    reason = "integration tests link the library crate and do not use its direct dependencies"
)]

use ironrdp_capture_replay::{
    Plaintext, ReplayError, RpchChannel, extract_rpch_tunneled_rdp, pair_rpch_channels, rpch_channel,
};

const COMMON_HEADER: usize = 16;
const RESPONSE_HEADER: usize = COMMON_HEADER + 8;
const REQUEST_HEADER: usize = COMMON_HEADER + 8;
const PTYPE_REQUEST: u8 = 0;
const PTYPE_RESPONSE: u8 = 2;
const PFC_FIRST_FRAG: u8 = 0x01;
const PFC_LAST_FRAG: u8 = 0x02;
const SEND_TO_SERVER_OPNUM: u16 = 9;

fn common_header(ptype: u8, flags: u8, call_id: u32, fragment_length: usize) -> Vec<u8> {
    let mut pdu = vec![5, 0, ptype, flags, 0x10, 0, 0, 0];
    pdu.extend_from_slice(
        &u16::try_from(fragment_length)
            .expect("fragment length fits u16")
            .to_le_bytes(),
    );
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&call_id.to_le_bytes());
    pdu
}

fn request_pdu_with_flags(call_id: u32, opnum: u16, stub: &[u8], flags: u8) -> Vec<u8> {
    let fragment_length = REQUEST_HEADER + stub.len();
    let mut pdu = common_header(PTYPE_REQUEST, flags, call_id, fragment_length);
    pdu.extend_from_slice(&u32::try_from(stub.len()).expect("stub length fits u32").to_le_bytes());
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&opnum.to_le_bytes());
    pdu.extend_from_slice(stub);
    pdu
}

fn request_pdu(call_id: u32, opnum: u16, stub: &[u8]) -> Vec<u8> {
    request_pdu_with_flags(call_id, opnum, stub, PFC_FIRST_FRAG | PFC_LAST_FRAG)
}

fn response_pdu(call_id: u32, stub: &[u8]) -> Vec<u8> {
    let fragment_length = RESPONSE_HEADER + stub.len();
    let mut pdu = common_header(PTYPE_RESPONSE, PFC_FIRST_FRAG | PFC_LAST_FRAG, call_id, fragment_length);
    pdu.extend_from_slice(&u32::try_from(stub.len()).expect("stub length fits u32").to_le_bytes());
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&[0, 0]);
    pdu.extend_from_slice(stub);
    pdu
}

fn send_to_server_stub(buffers: &[&[u8]]) -> Vec<u8> {
    let mut stub = vec![0u8; 20];
    let total: usize = buffers.iter().map(|buffer| buffer.len()).sum::<usize>() + 4 * buffers.len();
    stub.extend_from_slice(&u32::try_from(total).expect("total length fits u32").to_le_bytes());
    stub.extend_from_slice(
        &u32::try_from(buffers.len())
            .expect("buffer count fits u32")
            .to_le_bytes(),
    );
    for buffer in buffers {
        stub.extend_from_slice(
            &u32::try_from(buffer.len())
                .expect("buffer length fits u32")
                .to_le_bytes(),
        );
    }
    for buffer in buffers {
        stub.extend_from_slice(buffer);
    }
    stub
}

fn in_channel(body: Vec<u8>) -> Plaintext {
    let mut client = b"RPC_IN_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\nContent-Length: 1073741824\r\n\r\n".to_vec();
    client.extend(body);
    Plaintext {
        client: vec![(1, client)],
        server: Vec::new(),
    }
}

fn out_channel(body: Vec<u8>) -> Plaintext {
    let mut server = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    server.extend(body);
    Plaintext {
        client: vec![(1, b"RPC_OUT_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\n\r\n".to_vec())],
        server: vec![(2, server)],
    }
}

#[test]
fn detects_rpch_in_and_out_channels() {
    let input = in_channel(Vec::new());
    let output = out_channel(Vec::new());

    assert_eq!(rpch_channel(&input), Some(RpchChannel::In));
    assert_eq!(rpch_channel(&output), Some(RpchChannel::Out));
    assert_eq!(
        rpch_channel(&Plaintext {
            client: vec![(1, b"RDG_OUT_DATA /remoteDesktopGateway/ HTTP/1.1\r\n\r\n".to_vec())],
            server: Vec::new(),
        }),
        None
    );
}

#[test]
fn pairs_in_and_out_channels() {
    let input = in_channel(Vec::new());
    let output = out_channel(Vec::new());
    let flows = [input, output];

    let pair = pair_rpch_channels(&flows).unwrap().unwrap();

    assert_eq!(rpch_channel(pair.0), Some(RpchChannel::In));
    assert_eq!(rpch_channel(pair.1), Some(RpchChannel::Out));
}

#[test]
fn rejects_a_single_rpch_channel() {
    let input = in_channel(Vec::new());

    assert!(matches!(
        pair_rpch_channels(&[input]),
        Err(ReplayError::GatewayFraming(_))
    ));
}

#[test]
fn ignores_non_rpch_flows_when_pairing() {
    let other = Plaintext {
        client: vec![(1, b"GET /kdcproxy HTTP/1.1\r\n\r\n".to_vec())],
        server: Vec::new(),
    };

    assert_eq!(pair_rpch_channels(&[other]).unwrap(), None);
}

#[test]
fn extracts_send_to_server_and_receive_pipe_payloads() {
    let rdp = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0];
    let cc = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0];
    let input = in_channel(request_pdu(7, SEND_TO_SERVER_OPNUM, &send_to_server_stub(&[&rdp])));
    let output = out_channel(response_pdu(6, &cc));

    let inner = extract_rpch_tunneled_rdp(&input, &output).unwrap();

    assert_eq!(inner.client, vec![(7, rdp.to_vec())]);
    assert_eq!(inner.server, vec![(6, cc.to_vec())]);
}

#[test]
fn skips_non_data_requests_and_control_responses() {
    let rdp = [9u8; 8];
    let cc = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0];
    let mut client_body = request_pdu(1, 1, &[0u8; 32]);
    client_body.extend(request_pdu(2, SEND_TO_SERVER_OPNUM, &send_to_server_stub(&[&rdp])));
    let mut server_body = response_pdu(1, &[0u8; 16]);
    server_body.extend(response_pdu(6, &cc));
    server_body.extend(response_pdu(7, &[0u8; 40]));
    let data = [0x03, 0x00, 0x00, 0x20, 0x02, 0xf0];
    server_body.extend(response_pdu(6, &data));

    let inner = extract_rpch_tunneled_rdp(&in_channel(client_body), &out_channel(server_body)).unwrap();

    assert_eq!(inner.client, vec![(2, rdp.to_vec())]);
    assert_eq!(inner.server, vec![(6, cc.to_vec()), (6, data.to_vec())]);
}

#[test]
fn strips_authentication_round_trips() {
    let rdp = [1u8, 2, 3];
    let mut client = b"RPC_IN_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\nContent-Length: 0\r\n\r\n".to_vec();
    client.extend(b"RPC_IN_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\nContent-Length: 1073741824\r\n\r\n");
    client.extend(request_pdu(1, SEND_TO_SERVER_OPNUM, &send_to_server_stub(&[&rdp])));
    let input = Plaintext {
        client: vec![(1, client)],
        server: Vec::new(),
    };
    let mut server = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 13\r\n\r\nAccess Denied".to_vec();
    server.extend(b"HTTP/1.1 200 OK\r\n\r\n");
    server.extend(response_pdu(1, &[0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0]));
    let output = Plaintext {
        client: vec![(1, b"RPC_OUT_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\n\r\n".to_vec())],
        server: vec![(2, server)],
    };

    let inner = extract_rpch_tunneled_rdp(&input, &output).unwrap();

    assert_eq!(inner.client, vec![(1, rdp.to_vec())]);
    assert_eq!(inner.server, vec![(1, vec![0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0])]);
}

#[test]
fn reassembles_fragmented_send_to_server_stubs() {
    let rdp = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0];
    let stub = send_to_server_stub(&[&rdp]);
    let split = stub.len() / 2;
    let mut body = request_pdu_with_flags(4, SEND_TO_SERVER_OPNUM, &stub[..split], PFC_FIRST_FRAG);
    body.extend(request_pdu_with_flags(
        4,
        SEND_TO_SERVER_OPNUM,
        &stub[split..],
        PFC_LAST_FRAG,
    ));
    let cc = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0];

    let inner = extract_rpch_tunneled_rdp(&in_channel(body), &out_channel(response_pdu(6, &cc))).unwrap();

    assert_eq!(inner.client, vec![(4, rdp.to_vec())]);
}

#[test]
fn rejects_a_tunnel_without_rdp_payloads() {
    let input = in_channel(request_pdu(1, 1, &[0u8; 32]));
    let output = out_channel(response_pdu(1, &[0u8; 16]));

    assert!(matches!(
        extract_rpch_tunneled_rdp(&input, &output),
        Err(ReplayError::MissingRdpState)
    ));
}
