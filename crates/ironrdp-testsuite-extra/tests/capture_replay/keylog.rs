use super::capture_replay_binary;

use std::path::{Path, PathBuf};
use std::process::Command;

use aes_gcm::aead::{AeadInPlace as _, KeyInit as _};
use aes_gcm::{Aes128Gcm, Nonce};
use hmac::{Hmac, Mac};
use ironrdp_capture_replay::{ReplayError, decrypt_tls, read_capture, replay_capture};
use sha2::Sha256;

fn temporary_path(name: &str) -> PathBuf {
    let thread_name = std::thread::current().name().unwrap_or("test").replace(':', "_");

    std::env::temp_dir().join(format!(
        "ironrdp-capture-replay-{name}-{}-{}.pcapng",
        std::process::id(),
        thread_name
    ))
}

fn write_capture(path: &Path, client_stream: &[u8], server_stream: &[u8]) {
    write_capture_to_server(path, 2, client_stream, server_stream);
}

fn write_capture_to_server(path: &Path, server_port: u16, client_stream: &[u8], server_stream: &[u8]) {
    let capture = pcapng([
        ethernet_tcp(1, 2, 1, server_port, 100, 0x02, &[]),
        ethernet_tcp(2, 1, server_port, 1, 200, 0x12, &[]),
        ethernet_tcp(1, 2, 1, server_port, 101, 0x10, client_stream),
        ethernet_tcp(2, 1, server_port, 1, 201, 0x10, server_stream),
    ]);
    std::fs::write(path, capture).expect("write synthetic capture");
}

fn x224_connection(code: u8) -> Vec<u8> {
    vec![3, 0, 0, 19, 14, code, 0, 0, 0, 0, 0, 1, 0, 8, 0, 1, 0, 0, 0]
}

fn pcapng(frames: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    let mut capture = block(
        0x0A0D_0D0A,
        &[
            0x4d, 0x3c, 0x2b, 0x1a, // byte-order magic
            1, 0, 0, 0, // version
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // unknown section length
        ],
    );
    capture.extend(block(1, &[1, 0, 0, 0, 0xff, 0xff, 0, 0]));
    for frame in frames {
        let length = u32::try_from(frame.len()).expect("frame length fits u32");
        let mut body = Vec::new();
        body.extend(0u32.to_le_bytes()); // interface ID
        body.extend(0u32.to_le_bytes()); // timestamp high
        body.extend(0u32.to_le_bytes()); // timestamp low
        body.extend(length.to_le_bytes());
        body.extend(length.to_le_bytes());
        body.extend(&frame);
        body.resize((body.len() + 3) & !3, 0);
        capture.extend(block(6, &body));
    }
    capture
}

fn block(kind: u32, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len() + 12).expect("pcapng block length fits u32");
    let mut block = Vec::new();
    block.extend(kind.to_le_bytes());
    block.extend(length.to_le_bytes());
    block.extend(body);
    block.extend(length.to_le_bytes());
    block
}

fn ethernet_tcp(
    source_host: u8,
    destination_host: u8,
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = vec![0; 14];
    frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());

    let total_length = u16::try_from(20 + 20 + payload.len()).expect("IPv4 packet length fits u16");
    frame.extend([
        0x45,
        0, // IPv4 version and header length, DSCP
        total_length.to_be_bytes()[0],
        total_length.to_be_bytes()[1],
        0,
        0,
        0,
        0,
        64,
        6,
        0,
        0, // identification, flags, TTL, protocol, checksum
        127,
        0,
        0,
        source_host,
        127,
        0,
        0,
        destination_host,
    ]);

    frame.extend(source_port.to_be_bytes());
    frame.extend(destination_port.to_be_bytes());
    frame.extend(sequence.to_be_bytes());
    frame.extend(0u32.to_be_bytes());
    frame.extend([0x50, flags]);
    frame.extend([0, 0, 0, 0, 0, 0]);
    frame.extend(payload);
    frame
}

fn tls_record(content_type: u8, body: &[u8]) -> Vec<u8> {
    let length = u16::try_from(body.len()).expect("TLS record length fits u16");
    let mut record = vec![content_type, 3, 3];
    record.extend(length.to_be_bytes());
    record.extend(body);
    record
}

fn handshake_record(kind: u8, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len()).expect("handshake length fits u32");
    let mut handshake = vec![kind, 0, 0, 0];
    handshake[1..].copy_from_slice(&length.to_be_bytes()[1..]);
    handshake.extend(body);
    tls_record(22, &handshake)
}

fn tls12_key_block(master: &[u8], client_random: &[u8], server_random: &[u8]) -> Vec<u8> {
    let mut seed = Vec::<u8>::new();
    seed.extend(server_random);
    seed.extend(client_random);
    let mut label_seed = b"key expansion".to_vec();
    label_seed.extend(seed);
    let mut a = <Hmac<Sha256> as Mac>::new_from_slice(master)
        .expect("valid HMAC key")
        .chain_update(&label_seed)
        .finalize()
        .into_bytes()
        .to_vec();
    let mut key_block = Vec::new();
    while key_block.len() < 40 {
        let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(master).expect("valid HMAC key");
        hmac.update(&a);
        hmac.update(&label_seed);
        key_block.extend(hmac.finalize().into_bytes());
        a = <Hmac<Sha256> as Mac>::new_from_slice(master)
            .expect("valid HMAC key")
            .chain_update(&a)
            .finalize()
            .into_bytes()
            .to_vec();
    }
    key_block.truncate(40);
    key_block
}

fn encrypt_tls12_record(key: &[u8], fixed_iv: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let explicit_iv = [7; 8];
    let mut nonce = [0; 12];
    nonce[..4].copy_from_slice(fixed_iv);
    nonce[4..].copy_from_slice(&explicit_iv);
    let mut additional_data = Vec::new();
    additional_data.extend(0u64.to_be_bytes());
    additional_data.push(23);
    additional_data.extend([3, 3]);
    additional_data.extend(
        u16::try_from(plaintext.len())
            .expect("plaintext length fits u16")
            .to_be_bytes(),
    );
    let mut ciphertext = plaintext.to_vec();
    let tag = Aes128Gcm::new_from_slice(key)
        .expect("valid AES-128 key")
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut ciphertext)
        .expect("encrypt synthetic TLS record");
    let mut body = explicit_iv.to_vec();
    body.extend(ciphertext);
    body.extend(tag);
    tls_record(23, &body)
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
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
    let key_block = tls12_key_block(&session.master_secret, &session.client_random, &session.server_random);
    let (client_key, rest) = key_block.split_at(16);
    let (server_key, rest) = rest.split_at(16);
    let (client_iv, server_iv) = rest.split_at(4);
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
    client_stream.extend(encrypt_tls12_record(client_key, client_iv, session.client_payload));
    let mut server_stream = session.server_prefix.to_vec();
    server_stream.extend(handshake_record(2, &server_hello));
    server_stream.extend(tls_record(20, &[1]));
    server_stream.extend(encrypt_tls12_record(server_key, server_iv, session.server_payload));
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
fn appends_external_nss_key_log() {
    let capture_path = temporary_path("external-key-log");
    write_capture(&capture_path, &x224_connection(0xe0), &x224_connection(0xd0));
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
fn decrypts_resumed_tls12_with_external_key_log() {
    let capture_path = temporary_path("decrypt-external-key-log");
    let (client_stream, server_stream, key_log) = tls12_streams(Tls12Session {
        client_random: [1; 32],
        server_random: [2; 32],
        master_secret: [3; 48],
        session_id: &[3, 4, 5, 6],
        client_prefix: &x224_connection(0xe0),
        server_prefix: &x224_connection(0xd0),
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
fn decrypts_gateway_tunnel_with_external_key_log() {
    let capture_path = temporary_path("gateway-external-key-log");
    let (inner_client, inner_server, inner_key_log) = tls12_streams(Tls12Session {
        client_random: [4; 32],
        server_random: [5; 32],
        master_secret: [6; 48],
        session_id: &[],
        client_prefix: &x224_connection(0xe0),
        server_prefix: &x224_connection(0xd0),
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
        replay_capture(&capture),
        Err(ReplayError::MissingTunneledTlsSecret)
    ));

    capture.add_tls_key_log(&inner_key_log);
    assert!(matches!(replay_capture(&capture), Err(ReplayError::MissingRdpState)));

    std::fs::remove_file(capture_path).expect("remove synthetic gateway capture");
}

#[test]
fn reports_invalid_key_log_file() {
    let capture_path = temporary_path("invalid-key-log");
    let key_log_path = capture_path.with_extension("log");
    write_capture(&capture_path, &x224_connection(0xe0), &x224_connection(0xd0));
    std::fs::write(&key_log_path, [0xff]).expect("write invalid key log");

    let output = Command::new(capture_replay_binary())
        .args([
            "--keylog",
            key_log_path.to_str().expect("UTF-8 path"),
            capture_path.to_str().expect("UTF-8 path"),
            capture_path.with_extension("output").to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("run replay binary");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("replay export failed: failed to read"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    std::fs::remove_file(capture_path).expect("remove synthetic capture");
    std::fs::remove_file(key_log_path).expect("remove synthetic key log");
}
