use aes_gcm::aead::{AeadInPlace as _, KeyInit as _};
use aes_gcm::{Aes128Gcm, Nonce};
use hmac::{Hmac, Mac};
use ironrdp_capture::{CaptureError, decrypt_tls};
use sha2::Sha256;

use super::transport::{confirm, read, request};

const TLS_VERSION_1_2: u16 = 0x0303;

#[test]
// oracle: spec
fn reports_standard_security_for_direct_rdp_without_tls() {
    let capture = read(
        "standard-security",
        [
            super::transport::ethernet_tcp(1, 2, 100, 0x02, &[]),
            super::transport::ethernet_tcp(1, 2, 101, 0x10, &request()),
            super::transport::ethernet_tcp(2, 1, 200, 0x10, &confirm()),
        ],
        None,
    );

    assert!(matches!(decrypt_tls(&capture), Err(CaptureError::StandardSecurity)));
}

#[test]
// oracle: reference
fn rejects_tls_without_matching_key_log_entry() {
    let client_hello = hello(1, [1; 32], None);
    let server_hello = hello(2, [3; 32], Some(0x009c));
    let capture = read(
        "missing-key-log",
        [
            super::transport::ethernet_tcp(1, 2, 100, 0x02, &[]),
            super::transport::ethernet_tcp(
                1,
                2,
                101,
                0x10,
                &[request().as_slice(), handshake_record(1, &client_hello).as_slice()].concat(),
            ),
            super::transport::ethernet_tcp(
                2,
                1,
                200,
                0x10,
                &[confirm().as_slice(), handshake_record(2, &server_hello).as_slice()].concat(),
            ),
        ],
        None,
    );

    assert!(matches!(decrypt_tls(&capture), Err(CaptureError::MissingTlsSecret)));
}

#[test]
// oracle: regression
fn ignores_a_truncated_trailing_tls_record() {
    let client_hello = hello(1, [1; 32], None);
    let server_hello = hello(2, [3; 32], Some(0x009c));
    let capture = read(
        "truncated-tls-record",
        [
            super::transport::ethernet_tcp(1, 2, 100, 0x02, &[]),
            super::transport::ethernet_tcp(
                1,
                2,
                101,
                0x10,
                &[
                    request().as_slice(),
                    handshake_record(1, &client_hello).as_slice(),
                    &[22, 3, 3, 0, 1],
                ]
                .concat(),
            ),
            super::transport::ethernet_tcp(
                2,
                1,
                200,
                0x10,
                &[confirm().as_slice(), handshake_record(2, &server_hello).as_slice()].concat(),
            ),
        ],
        None,
    );

    assert!(matches!(decrypt_tls(&capture), Err(CaptureError::MissingTlsSecret)));
}

#[test]
// oracle: reference
fn decrypts_tls13_handshake_and_application_records() {
    let client_random = [1; 32];
    let secrets = [
        ("CLIENT_HANDSHAKE_TRAFFIC_SECRET", [2; 32]),
        ("SERVER_HANDSHAKE_TRAFFIC_SECRET", [3; 32]),
        ("CLIENT_TRAFFIC_SECRET_0", [4; 32]),
        ("SERVER_TRAFFIC_SECRET_0", [5; 32]),
    ];
    let key_log = secrets
        .iter()
        .map(|(label, secret)| format!("{label} {} {}", hex(&client_random), hex(secret)))
        .collect::<Vec<_>>()
        .join("\n");
    let client_hello = hello(1, client_random, None);
    let server_hello = hello(2, [6; 32], Some(0x1301));
    let finished = [20, 0, 0, 0];
    let client_handshake = traffic_key(&[2; 32]);
    let server_handshake = traffic_key(&[3; 32]);
    let client_application = traffic_key(&[4; 32]);
    let server_application = traffic_key(&[5; 32]);

    let capture = read(
        "tls13",
        [
            super::transport::ethernet_tcp(1, 2, 100, 0x02, &[]),
            super::transport::ethernet_tcp(
                1,
                2,
                101,
                0x10,
                &[
                    request().as_slice(),
                    handshake_record(1, &client_hello).as_slice(),
                    encrypt_tls13_record(&client_handshake, 0, &finished, 22).as_slice(),
                    encrypt_tls13_record(&client_application, 0, b"client", 23).as_slice(),
                ]
                .concat(),
            ),
            super::transport::ethernet_tcp(
                2,
                1,
                200,
                0x10,
                &[
                    confirm().as_slice(),
                    handshake_record(2, &server_hello).as_slice(),
                    encrypt_tls13_record(&server_handshake, 0, &finished, 22).as_slice(),
                    encrypt_tls13_record(&server_application, 0, b"server", 23).as_slice(),
                ]
                .concat(),
            ),
        ],
        Some(&key_log),
    );

    let plaintext = decrypt_tls(&capture).expect("decrypt TLS 1.3 capture");

    assert_eq!(plaintext.client, vec![(2, b"client".to_vec())]);
    assert_eq!(plaintext.server, vec![(3, b"server".to_vec())]);
}

#[test]
// oracle: reference
fn decrypts_tls12_application_data() {
    let client_random = [1; 32];
    let server_random = [3; 32];
    let master = [2; 48];
    let key_log = format!("CLIENT_RANDOM {} {}", hex(&client_random), hex(&master));
    let client_hello = hello(1, client_random, None);
    let server_hello = hello(2, server_random, Some(0x009c));
    let (client_key, server_key, client_iv, server_iv) = tls12_keys(&master, &client_random, &server_random);

    let capture = read(
        "tls12",
        [
            super::transport::ethernet_tcp(1, 2, 100, 0x02, &[]),
            super::transport::ethernet_tcp(
                1,
                2,
                101,
                0x10,
                &[
                    request().as_slice(),
                    handshake_record(1, &client_hello).as_slice(),
                    change_cipher_spec().as_slice(),
                    encrypt_tls12_record(&client_key, client_iv, 0, b"client").as_slice(),
                ]
                .concat(),
            ),
            super::transport::ethernet_tcp(
                2,
                1,
                200,
                0x10,
                &[
                    confirm().as_slice(),
                    handshake_record(2, &server_hello).as_slice(),
                    change_cipher_spec().as_slice(),
                    encrypt_tls12_record(&server_key, server_iv, 0, b"server").as_slice(),
                ]
                .concat(),
            ),
        ],
        Some(&key_log),
    );

    let plaintext = decrypt_tls(&capture).expect("decrypt TLS 1.2 capture");

    assert_eq!(plaintext.client, vec![(2, b"client".to_vec())]);
    assert_eq!(plaintext.server, vec![(3, b"server".to_vec())]);
}

fn hello(kind: u8, random: [u8; 32], suite: Option<u16>) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(TLS_VERSION_1_2.to_be_bytes());
    body.extend(random);
    if kind == 2 {
        body.push(0);
        body.extend(suite.expect("server hello includes a cipher suite").to_be_bytes());
        body.push(0);
    }
    body
}

fn handshake_record(kind: u8, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len()).expect("TLS handshake fits u32");
    let mut handshake = vec![kind, 0, 0, 0];
    handshake[1..4].copy_from_slice(&length.to_be_bytes()[1..]);
    handshake.extend(body);
    tls_record(22, &handshake)
}

fn change_cipher_spec() -> Vec<u8> {
    tls_record(20, &[1])
}

fn tls_record(content_type: u8, body: &[u8]) -> Vec<u8> {
    let length = u16::try_from(body.len()).expect("TLS record fits u16");
    let mut record = vec![content_type];
    record.extend(TLS_VERSION_1_2.to_be_bytes());
    record.extend(length.to_be_bytes());
    record.extend(body);
    record
}

fn traffic_key(secret: &[u8; 32]) -> ([u8; 16], [u8; 12]) {
    let key = expand_label(secret, b"key", 16)
        .try_into()
        .expect("TLS key has 16 bytes");
    let iv = expand_label(secret, b"iv", 12).try_into().expect("TLS IV has 12 bytes");
    (key, iv)
}

fn expand_label(secret: &[u8; 32], label: &[u8], length: usize) -> Vec<u8> {
    let mut info = Vec::new();
    info.extend(u16::try_from(length).expect("length fits u16").to_be_bytes());
    info.push(u8::try_from(6 + label.len()).expect("label length fits u8"));
    info.extend(b"tls13 ");
    info.extend(label);
    info.push(0);
    let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC accepts key");
    hmac.update(&info);
    hmac.update(&[1]);
    hmac.finalize().into_bytes()[..length].to_vec()
}

fn encrypt_tls13_record(key: &([u8; 16], [u8; 12]), sequence: u64, content: &[u8], inner_type: u8) -> Vec<u8> {
    let mut plaintext = content.to_vec();
    plaintext.push(inner_type);
    let mut nonce = key.1;
    for (byte, sequence_byte) in nonce[4..].iter_mut().zip(sequence.to_be_bytes()) {
        *byte ^= sequence_byte;
    }
    let mut additional_data = vec![23];
    additional_data.extend(TLS_VERSION_1_2.to_be_bytes());
    additional_data.extend(
        u16::try_from(plaintext.len() + 16)
            .expect("TLS ciphertext length fits u16")
            .to_be_bytes(),
    );
    let tag = Aes128Gcm::new_from_slice(&key.0)
        .expect("AES key length is valid")
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext)
        .expect("encrypt TLS vector");
    plaintext.extend(tag);
    tls_record(23, &plaintext)
}

fn tls12_keys(
    master: &[u8; 48],
    client_random: &[u8; 32],
    server_random: &[u8; 32],
) -> ([u8; 16], [u8; 16], [u8; 4], [u8; 4]) {
    let mut seed = Vec::new();
    seed.extend(server_random);
    seed.extend(client_random);
    let key_block = tls_prf_sha256(master, b"key expansion", &seed, 40);
    (
        key_block[..16].try_into().expect("client key length"),
        key_block[16..32].try_into().expect("server key length"),
        key_block[32..36].try_into().expect("client IV length"),
        key_block[36..40].try_into().expect("server IV length"),
    )
}

fn tls_prf_sha256(secret: &[u8; 48], label: &[u8], seed: &[u8], length: usize) -> Vec<u8> {
    let mut label_seed = label.to_vec();
    label_seed.extend(seed);
    let mut a = hmac_sha256(secret, &label_seed);
    let mut output = Vec::new();
    while output.len() < length {
        let mut input = a.clone();
        input.extend(&label_seed);
        output.extend(hmac_sha256(secret, &input));
        a = hmac_sha256(secret, &a);
    }
    output.truncate(length);
    output
}

fn hmac_sha256(key: &[u8], input: &[u8]) -> Vec<u8> {
    let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts key");
    hmac.update(input);
    hmac.finalize().into_bytes().to_vec()
}

fn encrypt_tls12_record(key: &[u8; 16], fixed_iv: [u8; 4], sequence: u64, plaintext: &[u8]) -> Vec<u8> {
    let explicit = [7; 8];
    let mut nonce = [0; 12];
    nonce[..4].copy_from_slice(&fixed_iv);
    nonce[4..].copy_from_slice(&explicit);
    let mut additional_data = sequence.to_be_bytes().to_vec();
    additional_data.push(23);
    additional_data.extend(TLS_VERSION_1_2.to_be_bytes());
    additional_data.extend(
        u16::try_from(plaintext.len())
            .expect("TLS plaintext length fits u16")
            .to_be_bytes(),
    );
    let mut encrypted = plaintext.to_vec();
    let tag = Aes128Gcm::new_from_slice(key)
        .expect("AES key length is valid")
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut encrypted)
        .expect("encrypt TLS vector");
    let mut body = explicit.to_vec();
    body.extend(encrypted);
    body.extend(tag);
    tls_record(23, &body)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
