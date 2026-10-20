use aes_gcm::aead::{AeadInPlace as _, KeyInit as _};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce, Tag};
use hmac::{Hmac, Mac};
use sha2::{Sha256, Sha384};

use crate::transport::x224_connection_tpdu_end;
use crate::{Capture, CaptureError, PacketStream};

const TLS_CONTENT_CHANGE_CIPHER_SPEC: u8 = 20;
const TLS_CONTENT_HANDSHAKE: u8 = 22;
const TLS_CONTENT_APPLICATION_DATA: u8 = 23;
const TLS_VERSION_1_2: u16 = 0x0303;
const TLS_MAX_RECORD_LENGTH: usize = 16_384 + 256;

/// Decrypted TLS application data with packet provenance.
///
/// Callers must identify the negotiated security protocol before treating either stream as RDP framing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plaintext {
    /// Client-to-server application data.
    pub client: PacketStream,
    /// Server-to-client application data.
    pub server: PacketStream,
}

/// Decrypt TLS application data from a direct TCP capture.
///
/// TLS key-log entries are used only in memory and are not retained by the
/// returned streams.
pub fn decrypt_tls(capture: &Capture) -> Result<Plaintext, CaptureError> {
    decrypt_tls_streams(
        &capture.flow.client_stream,
        &capture.flow.server_stream,
        capture.tls_key_log.as_str(),
    )
}

/// Decrypt TLS application data from a pair of directional byte streams.
///
/// Used after gateway unwrap, when the inner RDP session is itself TLS.
pub(crate) fn decrypt_tls_streams(
    client_stream: &PacketStream,
    server_stream: &PacketStream,
    key_log: &str,
) -> Result<Plaintext, CaptureError> {
    let client_records = collect_tls_records(client_stream, 0xe0)?;
    let server_records = collect_tls_records(server_stream, 0xd0)?;
    let (client_records, server_records) = match (client_records, server_records) {
        (Some(client_records), Some(server_records)) => (client_records, server_records),
        (None, None) => return Err(CaptureError::StandardSecurity),
        _ => return Err(CaptureError::UnsupportedTls),
    };
    if client_records.is_empty() && server_records.is_empty() {
        return Err(CaptureError::StandardSecurity);
    }
    let tls = Tls::from_records(&client_records, &server_records, key_log)?;

    Ok(Plaintext {
        client: tls.decrypt(Direction::Client, &client_records)?,
        server: tls.decrypt(Direction::Server, &server_records)?,
    })
}

#[derive(Clone, Copy)]
enum Direction {
    Client,
    Server,
}

#[derive(Clone, Copy)]
enum Cipher {
    Aes128GcmSha256,
    Aes256GcmSha384,
}

enum Tls {
    V12(Tls12),
    V13(Tls13),
}

impl Tls {
    fn from_records(client: &PacketStream, server: &PacketStream, key_log: &str) -> Result<Self, CaptureError> {
        let client_hello = first_handshake(client, 1).ok_or(CaptureError::UnsupportedTls)?;
        let server_hello = first_handshake(server, 2).ok_or(CaptureError::UnsupportedTls)?;
        let suite = tls_cipher_suite(&server_hello)?;

        match suite {
            0x1301 | 0x1302 => Tls13::from_hello(&client_hello, suite, key_log).map(Self::V13),
            _ => Tls12::from_hello(&client_hello, &server_hello, suite, key_log).map(Self::V12),
        }
    }

    fn decrypt(&self, direction: Direction, records: &PacketStream) -> Result<PacketStream, CaptureError> {
        match self {
            Self::V12(tls) => tls.decrypt(direction, records),
            Self::V13(tls) => tls.decrypt(direction, records),
        }
    }
}

struct Tls12 {
    cipher: Cipher,
    client_key: Vec<u8>,
    server_key: Vec<u8>,
    client_iv: [u8; 4],
    server_iv: [u8; 4],
}

impl Tls12 {
    fn from_hello(client_hello: &[u8], server_hello: &[u8], suite: u16, key_log: &str) -> Result<Self, CaptureError> {
        if client_hello.len() < 34 || server_hello.len() < 38 {
            return Err(CaptureError::UnsupportedTls);
        }
        let client_random = &client_hello[2..34];
        let server_random = &server_hello[2..34];
        let cipher = match suite {
            0x009c | 0xc02b | 0xc02f => Cipher::Aes128GcmSha256,
            0x009d | 0xc02c | 0xc030 => Cipher::Aes256GcmSha384,
            _ => return Err(CaptureError::UnsupportedTls),
        };
        let master = parse_master_secret(key_log, client_random).ok_or(CaptureError::MissingTlsSecret)?;
        let mut seed = Vec::with_capacity(64);
        seed.extend_from_slice(server_random);
        seed.extend_from_slice(client_random);
        let key_len = match cipher {
            Cipher::Aes128GcmSha256 => 16,
            Cipher::Aes256GcmSha384 => 32,
        };
        let key_block_len = 2 * key_len + 8;
        let key_block = match cipher {
            Cipher::Aes128GcmSha256 => tls_prf_sha256(&master, b"key expansion", &seed, key_block_len)?,
            Cipher::Aes256GcmSha384 => tls_prf_sha384(&master, b"key expansion", &seed, key_block_len)?,
        };
        let (client_key, rest) = key_block.split_at(key_len);
        let (server_key, rest) = rest.split_at(key_len);
        let (client_iv, server_iv) = rest.split_at(4);

        Ok(Self {
            cipher,
            client_key: client_key.to_vec(),
            server_key: server_key.to_vec(),
            client_iv: client_iv.try_into().map_err(|_| CaptureError::UnsupportedTls)?,
            server_iv: server_iv.try_into().map_err(|_| CaptureError::UnsupportedTls)?,
        })
    }

    fn decrypt(&self, direction: Direction, records: &PacketStream) -> Result<PacketStream, CaptureError> {
        let mut encrypted = false;
        let mut sequence = 0u64;
        let mut output = Vec::new();
        for (packet, record) in records {
            let (content_type, version, body) = parse_tls_record(record).ok_or(CaptureError::UnsupportedTls)?;
            if content_type == TLS_CONTENT_CHANGE_CIPHER_SPEC {
                encrypted = true;
                sequence = 0;
                continue;
            }
            if !encrypted {
                continue;
            }
            let plaintext = self.decrypt_record(direction, sequence, content_type, version, body)?;
            sequence = sequence.checked_add(1).ok_or(CaptureError::UnsupportedTls)?;
            if content_type == TLS_CONTENT_APPLICATION_DATA {
                output.push((*packet, plaintext));
            }
        }
        Ok(output)
    }

    fn decrypt_record(
        &self,
        direction: Direction,
        sequence: u64,
        content_type: u8,
        version: u16,
        body: &[u8],
    ) -> Result<Vec<u8>, CaptureError> {
        if version != TLS_VERSION_1_2 || body.len() < 8 + 16 {
            return Err(CaptureError::UnsupportedTls);
        }
        let (explicit, ciphertext_and_tag) = body.split_at(8);
        let (ciphertext, tag_bytes) = ciphertext_and_tag.split_at(ciphertext_and_tag.len() - 16);
        let plaintext_len = u16::try_from(ciphertext.len()).map_err(|_| CaptureError::UnsupportedTls)?;
        let mut additional_data = Vec::with_capacity(13);
        additional_data.extend_from_slice(&sequence.to_be_bytes());
        additional_data.push(content_type);
        additional_data.extend_from_slice(&version.to_be_bytes());
        additional_data.extend_from_slice(&plaintext_len.to_be_bytes());
        let (key, fixed_iv) = match direction {
            Direction::Client => (&self.client_key, self.client_iv),
            Direction::Server => (&self.server_key, self.server_iv),
        };
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&fixed_iv);
        nonce[4..].copy_from_slice(explicit);
        let tag = Tag::from_slice(tag_bytes);
        let mut plaintext = ciphertext.to_vec();
        let result = match self.cipher {
            Cipher::Aes128GcmSha256 => Aes128Gcm::new_from_slice(key)
                .map_err(|_| CaptureError::UnsupportedTls)?
                .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
            Cipher::Aes256GcmSha384 => Aes256Gcm::new_from_slice(key)
                .map_err(|_| CaptureError::UnsupportedTls)?
                .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
        };
        result.map_err(|_| CaptureError::TlsAuthentication)?;
        Ok(plaintext)
    }
}

struct Tls13 {
    cipher: Cipher,
    client_handshake: TrafficKey,
    server_handshake: TrafficKey,
    client_application: TrafficKey,
    server_application: TrafficKey,
}

struct TrafficKey {
    key: Vec<u8>,
    iv: [u8; 12],
}

impl Tls13 {
    fn from_hello(client_hello: &[u8], suite: u16, key_log: &str) -> Result<Self, CaptureError> {
        let client_random = client_hello.get(2..34).ok_or(CaptureError::UnsupportedTls)?;
        let cipher = match suite {
            0x1301 => Cipher::Aes128GcmSha256,
            0x1302 => Cipher::Aes256GcmSha384,
            _ => return Err(CaptureError::UnsupportedTls),
        };
        let key_len = match cipher {
            Cipher::Aes128GcmSha256 => 16,
            Cipher::Aes256GcmSha384 => 32,
        };
        let client_handshake = tls13_traffic_key(
            cipher,
            parse_tls13_secret(key_log, "CLIENT_HANDSHAKE_TRAFFIC_SECRET", client_random)?,
            key_len,
        )?;
        let server_handshake = tls13_traffic_key(
            cipher,
            parse_tls13_secret(key_log, "SERVER_HANDSHAKE_TRAFFIC_SECRET", client_random)?,
            key_len,
        )?;
        let client_application = tls13_traffic_key(
            cipher,
            parse_tls13_secret(key_log, "CLIENT_TRAFFIC_SECRET_0", client_random)?,
            key_len,
        )?;
        let server_application = tls13_traffic_key(
            cipher,
            parse_tls13_secret(key_log, "SERVER_TRAFFIC_SECRET_0", client_random)?,
            key_len,
        )?;

        Ok(Self {
            cipher,
            client_handshake,
            server_handshake,
            client_application,
            server_application,
        })
    }

    fn decrypt(&self, direction: Direction, records: &PacketStream) -> Result<PacketStream, CaptureError> {
        let (handshake, application) = match direction {
            Direction::Client => (&self.client_handshake, &self.client_application),
            Direction::Server => (&self.server_handshake, &self.server_application),
        };
        let mut key = handshake;
        let mut sequence = 0u64;
        let mut output = Vec::new();
        let mut handshake_data = Vec::new();
        let mut handshake_phase = true;
        for (packet, record) in records {
            let (content_type, version, body) = parse_tls_record(record).ok_or(CaptureError::UnsupportedTls)?;
            if content_type != TLS_CONTENT_APPLICATION_DATA {
                continue;
            }
            let plaintext = decrypt_tls13_record(self.cipher, key, sequence, content_type, version, body)?;
            sequence = sequence.checked_add(1).ok_or(CaptureError::UnsupportedTls)?;
            let (content, inner_type) = tls13_inner_plaintext(&plaintext).ok_or(CaptureError::UnsupportedTls)?;
            if inner_type == TLS_CONTENT_HANDSHAKE && handshake_phase {
                handshake_data.extend_from_slice(content);
                if tls13_handshake_contains_finished(&handshake_data) {
                    key = application;
                    sequence = 0;
                    handshake_phase = false;
                }
            }
            if inner_type == TLS_CONTENT_HANDSHAKE && !handshake_phase && tls13_handshake_contains_key_update(content) {
                return Err(CaptureError::TlsKeyUpdate);
            }
            if inner_type == TLS_CONTENT_APPLICATION_DATA {
                output.push((*packet, content.to_vec()));
            }
        }
        Ok(output)
    }
}

fn collect_tls_records(stream: &PacketStream, x224_code: u8) -> Result<Option<PacketStream>, CaptureError> {
    let mut bytes = Vec::new();
    let mut packet_offsets = Vec::with_capacity(stream.len());
    for (packet, chunk) in stream {
        packet_offsets.push((bytes.len(), *packet));
        bytes.extend_from_slice(chunk);
    }
    let tpkt_end = x224_connection_tpdu_end(&bytes, x224_code);
    let start = if let Some(start) = tpkt_end {
        bytes
            .get(start..start + 3)
            .is_some_and(|header| header[0] == TLS_CONTENT_HANDSHAKE && header[1] == 3 && header[2] >= 1)
            .then_some(start)
    } else {
        bytes
            .windows(3)
            .position(|window| window[0] == TLS_CONTENT_HANDSHAKE && window[1] == 3 && window[2] >= 1)
    };
    let Some(mut offset) = start else {
        return Ok(None);
    };

    let mut records = Vec::new();
    while offset + 5 <= bytes.len() {
        let length = usize::from(u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]));
        if length > TLS_MAX_RECORD_LENGTH {
            return Err(CaptureError::UnsupportedTls);
        }
        let end = offset.checked_add(5 + length).ok_or(CaptureError::UnsupportedTls)?;
        if end > bytes.len() {
            break;
        }
        let packet_index = packet_offsets.partition_point(|(chunk_offset, _)| *chunk_offset <= offset) - 1;
        records.push((packet_offsets[packet_index].1, bytes[offset..end].to_vec()));
        offset = end;
    }
    Ok(Some(records))
}

fn parse_tls_record(record: &[u8]) -> Option<(u8, u16, &[u8])> {
    let header = record.get(..5)?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    let body = record.get(5..)?;
    (body.len() == length).then_some((header[0], u16::from_be_bytes([header[1], header[2]]), body))
}

fn first_handshake(records: &PacketStream, wanted_type: u8) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    for (_, record) in records {
        let (content_type, _, body) = parse_tls_record(record)?;
        if content_type == TLS_CONTENT_HANDSHAKE {
            data.extend_from_slice(body);
        }
    }
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let length = (usize::from(data[offset + 1]) << 16)
            | (usize::from(data[offset + 2]) << 8)
            | usize::from(data[offset + 3]);
        let end = offset.checked_add(4 + length)?;
        let body = data.get(offset + 4..end)?;
        if data[offset] == wanted_type {
            return Some(body.to_vec());
        }
        offset = end;
    }
    None
}

fn tls_cipher_suite(server_hello: &[u8]) -> Result<u16, CaptureError> {
    let session_len = usize::from(*server_hello.get(34).ok_or(CaptureError::UnsupportedTls)?);
    let suite_offset = 35usize.checked_add(session_len).ok_or(CaptureError::UnsupportedTls)?;
    server_hello
        .get(suite_offset..suite_offset + 2)
        .ok_or(CaptureError::UnsupportedTls)?
        .try_into()
        .map(u16::from_be_bytes)
        .map_err(|_| CaptureError::UnsupportedTls)
}

fn parse_master_secret(key_log: &str, client_random: &[u8]) -> Option<Vec<u8>> {
    let client_random = hex(client_random);
    key_log.lines().find_map(|line| {
        let mut fields = line.split_ascii_whitespace();
        (fields.next()? == "CLIENT_RANDOM" && fields.next()?.eq_ignore_ascii_case(&client_random))
            .then(|| decode_hex(fields.next()?))
            .flatten()
            .filter(|secret| secret.len() == 48)
    })
}

fn parse_tls13_secret(key_log: &str, label: &str, client_random: &[u8]) -> Result<Vec<u8>, CaptureError> {
    let client_random = hex(client_random);
    key_log
        .lines()
        .find_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            (fields.next()? == label && fields.next()?.eq_ignore_ascii_case(&client_random))
                .then(|| decode_hex(fields.next()?))
        })
        .flatten()
        .ok_or(CaptureError::MissingTlsSecret)
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

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    input.len().is_multiple_of(2).then_some(())?;
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from((high << 4) | low).ok()
        })
        .collect()
}

fn tls_prf_sha256(secret: &[u8], label: &[u8], seed: &[u8], length: usize) -> Result<Vec<u8>, CaptureError> {
    let mut label_seed = Vec::with_capacity(label.len() + seed.len());
    label_seed.extend_from_slice(label);
    label_seed.extend_from_slice(seed);
    let mut a = <Hmac<Sha256> as Mac>::new_from_slice(secret)
        .map_err(|_| CaptureError::UnsupportedTls)?
        .chain_update(&label_seed)
        .finalize()
        .into_bytes()
        .to_vec();
    let mut output = Vec::with_capacity(length);
    while output.len() < length {
        let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(secret).map_err(|_| CaptureError::UnsupportedTls)?;
        hmac.update(&a);
        hmac.update(&label_seed);
        output.extend_from_slice(&hmac.finalize().into_bytes());
        a = <Hmac<Sha256> as Mac>::new_from_slice(secret)
            .map_err(|_| CaptureError::UnsupportedTls)?
            .chain_update(&a)
            .finalize()
            .into_bytes()
            .to_vec();
    }
    output.truncate(length);
    Ok(output)
}

fn tls_prf_sha384(secret: &[u8], label: &[u8], seed: &[u8], length: usize) -> Result<Vec<u8>, CaptureError> {
    let mut label_seed = Vec::with_capacity(label.len() + seed.len());
    label_seed.extend_from_slice(label);
    label_seed.extend_from_slice(seed);
    let mut a = <Hmac<Sha384> as Mac>::new_from_slice(secret)
        .map_err(|_| CaptureError::UnsupportedTls)?
        .chain_update(&label_seed)
        .finalize()
        .into_bytes()
        .to_vec();
    let mut output = Vec::with_capacity(length);
    while output.len() < length {
        let mut hmac = <Hmac<Sha384> as Mac>::new_from_slice(secret).map_err(|_| CaptureError::UnsupportedTls)?;
        hmac.update(&a);
        hmac.update(&label_seed);
        output.extend_from_slice(&hmac.finalize().into_bytes());
        a = <Hmac<Sha384> as Mac>::new_from_slice(secret)
            .map_err(|_| CaptureError::UnsupportedTls)?
            .chain_update(&a)
            .finalize()
            .into_bytes()
            .to_vec();
    }
    output.truncate(length);
    Ok(output)
}

fn tls13_traffic_key(cipher: Cipher, secret: Vec<u8>, key_len: usize) -> Result<TrafficKey, CaptureError> {
    let key = tls13_expand_label(cipher, &secret, b"key", key_len)?;
    let iv = tls13_expand_label(cipher, &secret, b"iv", 12)?
        .try_into()
        .map_err(|_| CaptureError::UnsupportedTls)?;
    Ok(TrafficKey { key, iv })
}

fn tls13_expand_label(cipher: Cipher, secret: &[u8], label: &[u8], length: usize) -> Result<Vec<u8>, CaptureError> {
    let mut info = Vec::with_capacity(2 + 1 + 6 + label.len() + 1);
    info.extend_from_slice(
        &u16::try_from(length)
            .map_err(|_| CaptureError::UnsupportedTls)?
            .to_be_bytes(),
    );
    let label_len = 6usize.checked_add(label.len()).ok_or(CaptureError::UnsupportedTls)?;
    info.push(u8::try_from(label_len).map_err(|_| CaptureError::UnsupportedTls)?);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label);
    info.push(0);
    match cipher {
        Cipher::Aes128GcmSha256 => hkdf_expand_sha256(secret, &info, length),
        Cipher::Aes256GcmSha384 => hkdf_expand_sha384(secret, &info, length),
    }
}

fn hkdf_expand_sha256(secret: &[u8], info: &[u8], length: usize) -> Result<Vec<u8>, CaptureError> {
    let mut previous = Vec::new();
    let mut output = Vec::with_capacity(length);
    for counter in 1..=u8::MAX {
        let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(secret).map_err(|_| CaptureError::UnsupportedTls)?;
        hmac.update(&previous);
        hmac.update(info);
        hmac.update(&[counter]);
        previous = hmac.finalize().into_bytes().to_vec();
        let remaining = length.saturating_sub(output.len());
        output.extend_from_slice(&previous[..remaining.min(previous.len())]);
        if output.len() == length {
            return Ok(output);
        }
    }
    Err(CaptureError::UnsupportedTls)
}

fn hkdf_expand_sha384(secret: &[u8], info: &[u8], length: usize) -> Result<Vec<u8>, CaptureError> {
    let mut previous = Vec::new();
    let mut output = Vec::with_capacity(length);
    for counter in 1..=u8::MAX {
        let mut hmac = <Hmac<Sha384> as Mac>::new_from_slice(secret).map_err(|_| CaptureError::UnsupportedTls)?;
        hmac.update(&previous);
        hmac.update(info);
        hmac.update(&[counter]);
        previous = hmac.finalize().into_bytes().to_vec();
        let remaining = length.saturating_sub(output.len());
        output.extend_from_slice(&previous[..remaining.min(previous.len())]);
        if output.len() == length {
            return Ok(output);
        }
    }
    Err(CaptureError::UnsupportedTls)
}

fn decrypt_tls13_record(
    cipher: Cipher,
    key: &TrafficKey,
    sequence: u64,
    content_type: u8,
    version: u16,
    body: &[u8],
) -> Result<Vec<u8>, CaptureError> {
    if version != TLS_VERSION_1_2 || body.len() < 16 {
        return Err(CaptureError::UnsupportedTls);
    }
    let (ciphertext, tag_bytes) = body.split_at(body.len() - 16);
    let mut nonce = key.iv;
    for (byte, sequence_byte) in nonce[4..].iter_mut().zip(sequence.to_be_bytes()) {
        *byte ^= sequence_byte;
    }
    let mut additional_data = Vec::with_capacity(5);
    additional_data.push(content_type);
    additional_data.extend_from_slice(&version.to_be_bytes());
    additional_data.extend_from_slice(
        &u16::try_from(body.len())
            .map_err(|_| CaptureError::UnsupportedTls)?
            .to_be_bytes(),
    );
    let tag = Tag::from_slice(tag_bytes);
    let mut plaintext = ciphertext.to_vec();
    let result = match cipher {
        Cipher::Aes128GcmSha256 => Aes128Gcm::new_from_slice(&key.key)
            .map_err(|_| CaptureError::UnsupportedTls)?
            .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
        Cipher::Aes256GcmSha384 => Aes256Gcm::new_from_slice(&key.key)
            .map_err(|_| CaptureError::UnsupportedTls)?
            .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
    };
    result.map_err(|_| CaptureError::TlsAuthentication)?;
    Ok(plaintext)
}

fn tls13_inner_plaintext(plaintext: &[u8]) -> Option<(&[u8], u8)> {
    let content_end = plaintext.iter().rposition(|byte| *byte != 0)?;
    Some((&plaintext[..content_end], plaintext[content_end]))
}

fn tls13_handshake_contains_finished(data: &[u8]) -> bool {
    let mut offset = 0;
    while let Some(header) = data.get(offset..offset + 4) {
        let length = (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        let Some(end) = offset.checked_add(4 + length) else {
            return false;
        };
        if end > data.len() {
            return false;
        }

        if header[0] == 20 {
            return true;
        }
        offset = end;
    }
    false
}

fn tls13_handshake_contains_key_update(data: &[u8]) -> bool {
    let mut offset = 0;
    while let Some(header) = data.get(offset..offset + 4) {
        let length = (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        let Some(end) = offset.checked_add(4 + length) else {
            return false;
        };
        if end > data.len() {
            return false;
        }
        if header[0] == 24 {
            return true;
        }
        offset = end;
    }
    false
}
