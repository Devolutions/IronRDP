use std::path::{Path, PathBuf};

use ironrdp_capture::{CaptureError, decrypt_rdp, decrypt_tls, read_capture};
use ironrdp_capture_replay::{ReplayError, replay_capture};

use super::tls::{encrypt_tls12_record, handshake_record, hex, tls_record, tls12_keys};
use super::transport::{capture, confirm, ethernet_tcp, request};

fn temporary_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ironrdp-capture-keylog-{name}-{}.pcapng", std::process::id()))
}

fn write_capture(path: &Path, client_stream: &[u8], server_stream: &[u8]) {
    write_capture_to_server(path, 2, client_stream, server_stream);
}

fn write_capture_to_server(path: &Path, server_port: u16, client_stream: &[u8], server_stream: &[u8]) {
    let capture = capture(
        [
            ethernet_tcp(1, server_port, 100, 0x02, &[]),
            ethernet_tcp(server_port, 1, 200, 0x12, &[]),
            ethernet_tcp(1, server_port, 101, 0x10, client_stream),
            ethernet_tcp(server_port, 1, 201, 0x10, server_stream),
        ],
        None,
    );
    std::fs::write(path, capture).expect("write synthetic capture");
}

struct Tls12Session<'a> {
    client_random: [u8; 32],
    server_random: [u8; 32],
    master_secret: [u8; 48],
    session_id: &'a [u8],
    client_prefix: &'a [u8],
    server_prefix: &'a [u8],
    client_payload: &'a [u8],
    server_payload: &'a [u8],
}

fn tls12_streams(session: Tls12Session<'_>) -> (Vec<u8>, Vec<u8>, String) {
    let (client_key, server_key, client_iv, server_iv) =
        tls12_keys(&session.master_secret, &session.client_random, &session.server_random);
    let mut client_hello = vec![3, 3];
    client_hello.extend(session.client_random);
    let mut server_hello = vec![3, 3];
    server_hello.extend(session.server_random);
    server_hello.push(u8::try_from(session.session_id.len()).expect("session ID length fits u8"));
    server_hello.extend(session.session_id);
    server_hello.extend([0x00, 0x9c, 0]);
    let mut client_stream = session.client_prefix.to_vec();
    client_stream.extend(handshake_record(1, &client_hello));
    client_stream.extend(tls_record(20, &[1]));
    client_stream.extend(encrypt_tls12_record(&client_key, client_iv, 0, session.client_payload));
    let mut server_stream = session.server_prefix.to_vec();
    server_stream.extend(handshake_record(2, &server_hello));
    server_stream.extend(tls_record(20, &[1]));
    server_stream.extend(encrypt_tls12_record(&server_key, server_iv, 0, session.server_payload));
    let key_log = format!(
        "CLIENT_RANDOM {} {}",
        hex(&session.client_random).to_ascii_uppercase(),
        hex(&session.master_secret)
    );
    (client_stream, server_stream, key_log)
}

fn gateway_data_packet(data: &[u8]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend(0x0Au16.to_le_bytes());
    packet.extend(0u16.to_le_bytes());
    packet.extend(
        u32::try_from(10 + data.len())
            .expect("packet length fits u32")
            .to_le_bytes(),
    );
    packet.extend(u16::try_from(data.len()).expect("data length fits u16").to_le_bytes());
    packet.extend(data);
    packet
}

fn websocket_frame(data: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut frame = vec![0x82];
    let mask_bit = u8::from(mask.is_some()) << 7;
    if data.len() < 126 {
        frame.push(mask_bit | u8::try_from(data.len()).expect("short frame length fits u8"));
    } else {
        frame.push(mask_bit | 126);
        frame.extend(u16::try_from(data.len()).expect("frame length fits u16").to_be_bytes());
    }
    if let Some(mask) = mask {
        frame.extend(mask);
        frame.extend(data.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]));
    } else {
        frame.extend(data);
    }
    frame
}

#[test]
// oracle: scaffold
fn appends_external_nss_key_log() {
    let capture_path = temporary_path("external-key-log");
    write_capture(&capture_path, &request(), &confirm());
    let mut capture = read_capture(&capture_path).expect("read synthetic capture");

    capture.add_tls_key_log("CLIENT_RANDOM first");
    capture.add_tls_key_log("");
    capture.add_tls_key_log("CLIENT_RANDOM second");

    assert_eq!(
        capture.tls_key_log.as_str(),
        "CLIENT_RANDOM first\nCLIENT_RANDOM second\n"
    );
    std::fs::remove_file(capture_path).expect("remove synthetic capture");
}

#[test]
// oracle: reference
fn decrypts_resumed_tls12_with_external_key_log() {
    let capture_path = temporary_path("decrypt-external-key-log");
    let (client_stream, server_stream, key_log) = tls12_streams(Tls12Session {
        client_random: [1; 32],
        server_random: [2; 32],
        master_secret: [3; 48],
        session_id: &[3, 4, 5, 6],
        client_prefix: &request(),
        server_prefix: &confirm(),
        client_payload: b"client",
        server_payload: b"server",
    });
    write_capture(&capture_path, &client_stream, &server_stream);

    let mut capture = read_capture(&capture_path).expect("read synthetic capture");
    capture.add_tls_key_log(&key_log);

    let plaintext = decrypt_tls(&capture).expect("decrypt with external synthetic key log");
    assert_eq!(plaintext.client, vec![(3, b"client".to_vec())]);
    assert_eq!(plaintext.server, vec![(4, b"server".to_vec())]);

    std::fs::remove_file(capture_path).expect("remove synthetic capture");
}

#[test]
// oracle: regression
fn decrypts_gateway_tunnel_with_external_key_log() {
    let capture_path = temporary_path("gateway-external-key-log");
    let (inner_client, inner_server, inner_key_log) = tls12_streams(Tls12Session {
        client_random: [4; 32],
        server_random: [5; 32],
        master_secret: [6; 48],
        session_id: &[],
        client_prefix: &request(),
        server_prefix: &confirm(),
        client_payload: b"client",
        server_payload: b"server",
    });
    let mut gateway_client = b"RDG_OUT_DATA /remoteDesktopGateway/ HTTP/1.1\r\n\r\n".to_vec();
    gateway_client.extend(websocket_frame(&gateway_data_packet(&inner_client), Some([1, 2, 3, 4])));
    let mut gateway_server = b"HTTP/1.1 101 Switching Protocols\r\n\r\n".to_vec();
    gateway_server.extend(websocket_frame(&gateway_data_packet(&inner_server), None));
    let (outer_client, outer_server, outer_key_log) = tls12_streams(Tls12Session {
        client_random: [7; 32],
        server_random: [8; 32],
        master_secret: [9; 48],
        session_id: &[],
        client_prefix: &[],
        server_prefix: &[],
        client_payload: &gateway_client,
        server_payload: &gateway_server,
    });
    write_capture_to_server(&capture_path, 443, &outer_client, &outer_server);

    let mut capture = read_capture(&capture_path).expect("read synthetic gateway capture");
    capture.add_tls_key_log(&outer_key_log);
    assert!(matches!(
        decrypt_rdp(&capture),
        Err(CaptureError::MissingTunneledTlsSecret)
    ));

    capture.add_tls_key_log(&inner_key_log);
    assert!(matches!(replay_capture(&capture), Err(ReplayError::MissingRdpState)));

    std::fs::remove_file(capture_path).expect("remove synthetic gateway capture");
}
