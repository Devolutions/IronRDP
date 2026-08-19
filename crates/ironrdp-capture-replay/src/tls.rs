use aes_gcm::aead::{AeadInPlace as _, KeyInit as _};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce, Tag};
use hmac::{Hmac, Mac};
use sha2::{Sha256, Sha384};

use crate::transport::x224_connection_tpdu_end;
use crate::{Capture, PacketStream, ReplayError};

#[cfg(test)]
use crate::TlsKeyLog;

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
pub fn decrypt_tls(capture: &Capture) -> Result<Plaintext, ReplayError> {
    decrypt_tls_streams(
        &capture.flow.client_stream,
        &capture.flow.server_stream,
        capture.tls_key_log.as_str(),
    )
}

/// Decrypt TLS application data from a pair of directional byte streams.
///
/// Public for offline tooling that extracts inner streams (e.g. gateway tunnels)
/// before decryption.
pub fn decrypt_tls_streams(
    client_stream: &PacketStream,
    server_stream: &PacketStream,
    key_log: &str,
) -> Result<Plaintext, ReplayError> {
    let client_records = collect_tls_records(client_stream, 0xe0)?;
    let server_records = collect_tls_records(server_stream, 0xd0)?;
    let (client_records, server_records) = match (client_records, server_records) {
        (Some(client_records), Some(server_records)) => (client_records, server_records),
        (None, None) => return Err(ReplayError::StandardSecurity),
        _ => return Err(ReplayError::UnsupportedTls),
    };
    if client_records.is_empty() && server_records.is_empty() {
        return Err(ReplayError::StandardSecurity);
    }
    // Full-handshake sessions first; a mid-stream capture (tunneled session recorded
    // after the handshake, or with secrets logged only for the application phase) falls
    // back to application-secret-only decryption.
    let tls = Tls::from_records(&client_records, &server_records, key_log)
        .or_else(|_| Tls::from_records_midstream(&client_records, &server_records, key_log))?;

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
    fn from_records(client: &PacketStream, server: &PacketStream, key_log: &str) -> Result<Self, ReplayError> {
        let client_hello = first_handshake(client, 1).ok_or(ReplayError::UnsupportedTls)?;
        let server_hello = first_handshake(server, 2).ok_or(ReplayError::UnsupportedTls)?;
        let suite = tls_cipher_suite(&server_hello)?;

        match suite {
            0x1301 | 0x1302 => Tls13::from_hello(&client_hello, suite, key_log, client, server).map(Self::V13),
            _ => Tls12::from_hello(&client_hello, &server_hello, suite, key_log).map(Self::V12),
        }
    }

    /// Builds a TLS 1.3 decryptor for a mid-stream capture where the handshake is absent
    /// and only application-traffic secrets are logged.
    ///
    /// Tunneled sessions captured after the handshake carry no ClientHello/ServerHello; the
    /// session is identified by trying every logged application secret against the first
    /// application record.
    fn from_records_midstream(
        client: &PacketStream,
        server: &PacketStream,
        key_log: &str,
    ) -> Result<Self, ReplayError> {
        // Find each direction's first application-data record.
        let first_client = first_application_record(client).ok_or(ReplayError::UnsupportedTls)?;
        let first_server = first_application_record(server).ok_or(ReplayError::UnsupportedTls)?;
        let cipher = guess_tls13_cipher(&first_client, &first_server).ok_or(ReplayError::UnsupportedTls)?;

        Tls13::from_application_secrets(cipher, &first_client, &first_server, key_log).map(Self::V13)
    }

    fn decrypt(&self, direction: Direction, records: &PacketStream) -> Result<PacketStream, ReplayError> {
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
    fn from_hello(client_hello: &[u8], server_hello: &[u8], suite: u16, key_log: &str) -> Result<Self, ReplayError> {
        if client_hello.len() < 34 || server_hello.len() < 38 {
            return Err(ReplayError::UnsupportedTls);
        }
        let client_random = &client_hello[2..34];
        let server_random = &server_hello[2..34];
        let cipher = match suite {
            0x009c | 0xc02b | 0xc02f => Cipher::Aes128GcmSha256,
            0x009d | 0xc02c | 0xc030 => Cipher::Aes256GcmSha384,
            _ => return Err(ReplayError::UnsupportedTls),
        };
        let master = parse_master_secret(key_log, client_random).ok_or(ReplayError::MissingTlsSecret)?;
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
            client_iv: client_iv.try_into().map_err(|_| ReplayError::UnsupportedTls)?,
            server_iv: server_iv.try_into().map_err(|_| ReplayError::UnsupportedTls)?,
        })
    }

    fn decrypt(&self, direction: Direction, records: &PacketStream) -> Result<PacketStream, ReplayError> {
        let mut encrypted = false;
        let mut sequence = 0u64;
        let mut output = Vec::new();
        for (packet, record) in records {
            let (content_type, version, body) = parse_tls_record(record).ok_or(ReplayError::UnsupportedTls)?;
            if content_type == TLS_CONTENT_CHANGE_CIPHER_SPEC {
                encrypted = true;
                sequence = 0;
                continue;
            }
            if !encrypted {
                continue;
            }
            let plaintext = self.decrypt_record(direction, sequence, content_type, version, body)?;
            sequence = sequence.checked_add(1).ok_or(ReplayError::UnsupportedTls)?;
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
    ) -> Result<Vec<u8>, ReplayError> {
        if version != TLS_VERSION_1_2 || body.len() < 8 + 16 {
            return Err(ReplayError::UnsupportedTls);
        }
        let (explicit, ciphertext_and_tag) = body.split_at(8);
        let (ciphertext, tag_bytes) = ciphertext_and_tag.split_at(ciphertext_and_tag.len() - 16);
        let plaintext_len = u16::try_from(ciphertext.len()).map_err(|_| ReplayError::UnsupportedTls)?;
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
                .map_err(|_| ReplayError::UnsupportedTls)?
                .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
            Cipher::Aes256GcmSha384 => Aes256Gcm::new_from_slice(key)
                .map_err(|_| ReplayError::UnsupportedTls)?
                .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
        };
        result.map_err(|_| ReplayError::TlsAuthentication)?;
        Ok(plaintext)
    }
}

#[derive(Clone)]
struct Tls13 {
    cipher: Cipher,
    client_handshake: Option<TrafficKey>,
    server_handshake: Option<TrafficKey>,
    client_application: TrafficKey,
    server_application: TrafficKey,
    has_handshake_keys: bool,
}

#[derive(Clone)]
struct TrafficKey {
    key: Vec<u8>,
    iv: [u8; 12],
}

impl Tls13 {
    fn from_hello(
        client_hello: &[u8],
        suite: u16,
        key_log: &str,
        client_records: &PacketStream,
        server_records: &PacketStream,
    ) -> Result<Self, ReplayError> {
        let client_random = client_hello.get(2..34).ok_or(ReplayError::UnsupportedTls)?;
        let cipher = match suite {
            0x1301 => Cipher::Aes128GcmSha256,
            0x1302 => Cipher::Aes256GcmSha384,
            _ => return Err(ReplayError::UnsupportedTls),
        };
        let key_len = match cipher {
            Cipher::Aes128GcmSha256 => 16,
            Cipher::Aes256GcmSha384 => 32,
        };
        // Application secrets are always required. Handshake secrets may be absent when a
        // key log captured the session only after the handshake completed (common for
        // tunneled sessions logged by an out-of-band dumper); such sessions start
        // decryption at the application phase.
        let client_handshake = parse_tls13_secret(key_log, "CLIENT_HANDSHAKE_TRAFFIC_SECRET", client_random)
            .ok()
            .map(|secret| tls13_traffic_key(cipher, secret, key_len))
            .transpose()?;
        let server_handshake = parse_tls13_secret(key_log, "SERVER_HANDSHAKE_TRAFFIC_SECRET", client_random)
            .ok()
            .map(|secret| tls13_traffic_key(cipher, secret, key_len))
            .transpose()?;

        // A session's traffic secret can be logged several times under one client random
        // (key update, re-key), and an out-of-band dumper does not guarantee log order.
        // Score each candidate pair by how much of the session it actually decrypts and
        // keep the best; a wrong key yields no application output because the
        // handshake-tail skip consumes every record, while the right key decrypts the
        // full stream.
        let client_candidates = parse_tls13_secrets(key_log, "CLIENT_TRAFFIC_SECRET_0", client_random);
        let server_candidates = parse_tls13_secrets(key_log, "SERVER_TRAFFIC_SECRET_0", client_random);
        if client_candidates.is_empty() || server_candidates.is_empty() {
            return Err(ReplayError::MissingTlsSecret);
        }
        let mut first_candidate: Option<Self> = None;
        let mut best: Option<(usize, Self)> = None;
        for client_secret in &client_candidates {
            for server_secret in &server_candidates {
                let client_application = tls13_traffic_key(cipher, client_secret.clone(), key_len)?;
                let server_application = tls13_traffic_key(cipher, server_secret.clone(), key_len)?;
                let candidate = Self {
                    cipher,
                    client_handshake: client_handshake.clone(),
                    server_handshake: server_handshake.clone(),
                    client_application,
                    server_application,
                    has_handshake_keys: client_handshake.is_some() && server_handshake.is_some(),
                };
                // Kept as a fallback so a session with no application records yet still
                // builds a decryptor.
                if first_candidate.is_none() {
                    first_candidate = Some(candidate.clone());
                }
                let score = candidate.decrypted_len(Direction::Client, client_records)
                    + candidate.decrypted_len(Direction::Server, server_records);
                if score > 0 && best.as_ref().is_none_or(|(best_score, _)| score > *best_score) {
                    best = Some((score, candidate));
                }
            }
        }
        best.map(|(_, candidate)| candidate)
            .or(first_candidate)
            .ok_or(ReplayError::MissingTlsSecret)
    }

    /// Total application bytes a candidate decrypts in one direction, or 0 when the key
    /// does not fit (decryption error or empty output).
    fn decrypted_len(&self, direction: Direction, records: &PacketStream) -> usize {
        self.decrypt(direction, records)
            .map(|stream| stream.iter().map(|(_, chunk)| chunk.len()).sum())
            .unwrap_or(0)
    }

    /// Builds a decryptor from application-traffic secrets alone, for a mid-stream
    /// capture whose handshake (and client random) is not in the capture.
    ///
    /// The session is identified by deriving keys from each logged
    /// `CLIENT_TRAFFIC_SECRET_0`/`SERVER_TRAFFIC_SECRET_0` pair and authenticating the
    /// first application record; the GCM tag check is decisive.
    fn from_application_secrets(
        cipher: Cipher,
        first_client_record: &[u8],
        first_server_record: &[u8],
        key_log: &str,
    ) -> Result<Self, ReplayError> {
        let key_len = match cipher {
            Cipher::Aes128GcmSha256 => 16,
            Cipher::Aes256GcmSha384 => 32,
        };
        // Collect client randoms that have both application secrets logged.
        let mut client_randoms = Vec::new();
        for line in key_log.lines().map(|line| line.replace('\0', "")) {
            let mut fields = line.split_ascii_whitespace();
            if fields.next() == Some("CLIENT_TRAFFIC_SECRET_0")
                && let Some(random) = fields.next()
            {
                client_randoms.push(random.to_owned());
            }
        }
        client_randoms.sort_unstable();
        client_randoms.dedup();

        for random in client_randoms {
            let client_random_bytes = decode_hex(&random).ok_or(ReplayError::MissingTlsSecret)?;
            let Ok(client_application) = parse_tls13_secret(key_log, "CLIENT_TRAFFIC_SECRET_0", &client_random_bytes)
                .and_then(|secret| tls13_traffic_key(cipher, secret, key_len))
            else {
                continue;
            };
            let Ok(server_application) = parse_tls13_secret(key_log, "SERVER_TRAFFIC_SECRET_0", &client_random_bytes)
                .and_then(|secret| tls13_traffic_key(cipher, secret, key_len))
            else {
                continue;
            };

            // Try the candidate against both directions' first application records.
            let candidate = Self {
                cipher,
                client_handshake: None,
                server_handshake: None,
                client_application,
                server_application,
                has_handshake_keys: false,
            };
            if candidate
                .decrypt(Direction::Client, &vec![(0, first_client_record.to_vec())])
                .is_ok()
                && candidate
                    .decrypt(Direction::Server, &vec![(0, first_server_record.to_vec())])
                    .is_ok()
            {
                return Ok(candidate);
            }
        }
        Err(ReplayError::MissingTlsSecret)
    }

    fn decrypt(&self, direction: Direction, records: &PacketStream) -> Result<PacketStream, ReplayError> {
        // Without handshake secrets, decryption starts at the application phase.
        let (handshake, application) = match direction {
            Direction::Client => (&self.client_handshake, &self.client_application),
            Direction::Server => (&self.server_handshake, &self.server_application),
        };
        let application: &TrafficKey = application;
        let mut handshake_phase = self.has_handshake_keys;
        let mut key: &TrafficKey = handshake.as_ref().unwrap_or(application);
        let mut sequence = 0u64;
        let mut output = Vec::new();
        let mut handshake_data = Vec::new();
        for (packet, record) in records {
            let (content_type, version, body) = parse_tls_record(record).ok_or(ReplayError::UnsupportedTls)?;
            if content_type != TLS_CONTENT_APPLICATION_DATA {
                continue;
            }
            // When handshake secrets are absent, the opening application records may
            // still carry the handshake tail (encrypted with the unknown handshake keys);
            // skip records that fail authentication until the application phase starts.
            let plaintext = match decrypt_tls13_record(self.cipher, key, sequence, content_type, version, body) {
                Ok(plaintext) => plaintext,
                // A record that fails the application key before the application phase has
                // started is the handshake tail (EncryptedExtensions/Finished) encrypted
                // with the unknown handshake key. It does not consume an application
                // sequence number, so the sequence is left unchanged while skipping it.
                Err(ReplayError::TlsAuthentication) if !handshake_phase && output.is_empty() => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            sequence = sequence.checked_add(1).ok_or(ReplayError::UnsupportedTls)?;
            let (content, inner_type) = tls13_inner_plaintext(&plaintext).ok_or(ReplayError::UnsupportedTls)?;
            if inner_type == TLS_CONTENT_HANDSHAKE && handshake_phase {
                handshake_data.extend_from_slice(content);
                if tls13_handshake_contains_finished(&handshake_data) {
                    key = application;
                    sequence = 0;
                    handshake_phase = false;
                }
            }
            if inner_type == TLS_CONTENT_HANDSHAKE && !handshake_phase && tls13_handshake_contains_key_update(content) {
                return Err(ReplayError::TlsKeyUpdate);
            }
            if inner_type == TLS_CONTENT_APPLICATION_DATA {
                output.push((*packet, content.to_vec()));
            }
        }
        Ok(output)
    }
}

fn collect_tls_records(stream: &PacketStream, x224_code: u8) -> Result<Option<PacketStream>, ReplayError> {
    let mut bytes = Vec::new();
    let mut packet_offsets = Vec::with_capacity(stream.len());
    for (packet, chunk) in stream {
        packet_offsets.push((bytes.len(), *packet));
        bytes.extend_from_slice(chunk);
    }
    // A direct capture starts TLS immediately after the X.224 connection TPDU. A
    // gateway-unwrapped stream may pad between the TPDU and the first TLS record, so when
    // the bytes right after the TPDU are not a handshake record, scan for the first one.
    let scan = |bytes: &[u8]| {
        bytes
            .windows(3)
            .position(|window| window[0] == TLS_CONTENT_HANDSHAKE && window[1] == 3 && window[2] >= 1)
    };
    let start = match x224_connection_tpdu_end(&bytes, x224_code) {
        Some(tpkt_end)
            if bytes
                .get(tpkt_end..tpkt_end + 3)
                .is_some_and(|header| header[0] == TLS_CONTENT_HANDSHAKE && header[1] == 3 && header[2] >= 1) =>
        {
            Some(tpkt_end)
        }
        _ => scan(&bytes),
    };
    let Some(mut offset) = start else {
        return Ok(None);
    };

    let mut records = Vec::new();
    while offset + 5 <= bytes.len() {
        let length = usize::from(u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]));
        if length > TLS_MAX_RECORD_LENGTH {
            return Err(ReplayError::UnsupportedTls);
        }
        let end = offset.checked_add(5 + length).ok_or(ReplayError::UnsupportedTls)?;
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

/// First application-data record in a stream (header + ciphertext + tag), owned.
fn first_application_record(stream: &PacketStream) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for (_, chunk) in stream {
        bytes.extend_from_slice(chunk);
    }
    let mut offset = 0;
    while offset + 5 <= bytes.len() {
        if bytes[offset] != TLS_CONTENT_APPLICATION_DATA
            || bytes[offset + 1] != 3
            || !(1..=3).contains(&bytes[offset + 2])
        {
            offset += 1;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]));
        let end = offset.checked_add(5 + length)?;
        if end > bytes.len() {
            return None;
        }
        return Some(bytes[offset..end].to_vec());
    }
    None
}

/// Both directions of a mid-stream session share the cipher suite; guess it from the
/// record lengths (AES-256-GCM has a 32-byte key, AES-128-GCM a 16-byte key; the tag is
/// 16 bytes either way, so record size alone is ambiguous — prefer AES-256-GCM, the
/// common RD Gateway choice, and let the caller retry with the other).
fn guess_tls13_cipher(_client_record: &[u8], _server_record: &[u8]) -> Option<Cipher> {
    Some(Cipher::Aes256GcmSha384)
}

fn tls_cipher_suite(server_hello: &[u8]) -> Result<u16, ReplayError> {
    let session_len = usize::from(*server_hello.get(34).ok_or(ReplayError::UnsupportedTls)?);
    let suite_offset = 35usize.checked_add(session_len).ok_or(ReplayError::UnsupportedTls)?;
    server_hello
        .get(suite_offset..suite_offset + 2)
        .ok_or(ReplayError::UnsupportedTls)?
        .try_into()
        .map(u16::from_be_bytes)
        .map_err(|_| ReplayError::UnsupportedTls)
}

fn parse_master_secret(key_log: &str, client_random: &[u8]) -> Option<Vec<u8>> {
    let client_random = hex(client_random);
    // Same NUL tolerance as parse_tls13_secret for interleaved out-of-band logs.
    key_log.lines().map(|line| line.replace('\0', "")).find_map(|line| {
        let mut fields = line.split_ascii_whitespace();
        (fields.next()? == "CLIENT_RANDOM" && fields.next()?.eq_ignore_ascii_case(&client_random))
            .then(|| decode_hex(fields.next()?))
            .flatten()
            .filter(|secret| secret.len() == 48)
    })
}

fn parse_tls13_secret(key_log: &str, label: &str, client_random: &[u8]) -> Result<Vec<u8>, ReplayError> {
    parse_tls13_secrets(key_log, label, client_random)
        .into_iter()
        .next()
        .ok_or(ReplayError::MissingTlsSecret)
}

/// Collect every logged value of `label` for `client_random`.
///
/// An out-of-band LSA dumper can log a session's traffic secret more than once (for
/// example across a TLS key update, or when a connection is re-keyed), leaving several
/// distinct values under one client random. Callers must try each candidate and keep the
/// one that authenticates the captured records rather than trusting the first.
fn parse_tls13_secrets(key_log: &str, label: &str, client_random: &[u8]) -> Vec<Vec<u8>> {
    let client_random = hex(client_random);
    // Out-of-band key dumpers (for example an LSA SChannel logger writing from several
    // sessions at once) interleave NUL bytes into the log; drop them before parsing.
    key_log
        .lines()
        .map(|line| line.replace('\0', ""))
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            // Key logs are mixed-case across dumpers (Wireshark lowercases, some LSA
            // dumpers uppercase); match the client random case-insensitively.
            (fields.next()? == label && fields.next()?.eq_ignore_ascii_case(&client_random))
                .then(|| decode_hex(fields.next()?))
                .flatten()
        })
        .collect()
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

fn tls_prf_sha256(secret: &[u8], label: &[u8], seed: &[u8], length: usize) -> Result<Vec<u8>, ReplayError> {
    let mut label_seed = Vec::with_capacity(label.len() + seed.len());
    label_seed.extend_from_slice(label);
    label_seed.extend_from_slice(seed);
    let mut a = <Hmac<Sha256> as Mac>::new_from_slice(secret)
        .map_err(|_| ReplayError::UnsupportedTls)?
        .chain_update(&label_seed)
        .finalize()
        .into_bytes()
        .to_vec();
    let mut output = Vec::with_capacity(length);
    while output.len() < length {
        let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(secret).map_err(|_| ReplayError::UnsupportedTls)?;
        hmac.update(&a);
        hmac.update(&label_seed);
        output.extend_from_slice(&hmac.finalize().into_bytes());
        a = <Hmac<Sha256> as Mac>::new_from_slice(secret)
            .map_err(|_| ReplayError::UnsupportedTls)?
            .chain_update(&a)
            .finalize()
            .into_bytes()
            .to_vec();
    }
    output.truncate(length);
    Ok(output)
}

fn tls_prf_sha384(secret: &[u8], label: &[u8], seed: &[u8], length: usize) -> Result<Vec<u8>, ReplayError> {
    let mut label_seed = Vec::with_capacity(label.len() + seed.len());
    label_seed.extend_from_slice(label);
    label_seed.extend_from_slice(seed);
    let mut a = <Hmac<Sha384> as Mac>::new_from_slice(secret)
        .map_err(|_| ReplayError::UnsupportedTls)?
        .chain_update(&label_seed)
        .finalize()
        .into_bytes()
        .to_vec();
    let mut output = Vec::with_capacity(length);
    while output.len() < length {
        let mut hmac = <Hmac<Sha384> as Mac>::new_from_slice(secret).map_err(|_| ReplayError::UnsupportedTls)?;
        hmac.update(&a);
        hmac.update(&label_seed);
        output.extend_from_slice(&hmac.finalize().into_bytes());
        a = <Hmac<Sha384> as Mac>::new_from_slice(secret)
            .map_err(|_| ReplayError::UnsupportedTls)?
            .chain_update(&a)
            .finalize()
            .into_bytes()
            .to_vec();
    }
    output.truncate(length);
    Ok(output)
}

fn tls13_traffic_key(cipher: Cipher, secret: Vec<u8>, key_len: usize) -> Result<TrafficKey, ReplayError> {
    let key = tls13_expand_label(cipher, &secret, b"key", key_len)?;
    let iv = tls13_expand_label(cipher, &secret, b"iv", 12)?
        .try_into()
        .map_err(|_| ReplayError::UnsupportedTls)?;
    Ok(TrafficKey { key, iv })
}

fn tls13_expand_label(cipher: Cipher, secret: &[u8], label: &[u8], length: usize) -> Result<Vec<u8>, ReplayError> {
    let mut info = Vec::with_capacity(2 + 1 + 6 + label.len() + 1);
    info.extend_from_slice(
        &u16::try_from(length)
            .map_err(|_| ReplayError::UnsupportedTls)?
            .to_be_bytes(),
    );
    let label_len = 6usize.checked_add(label.len()).ok_or(ReplayError::UnsupportedTls)?;
    info.push(u8::try_from(label_len).map_err(|_| ReplayError::UnsupportedTls)?);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label);
    info.push(0);
    match cipher {
        Cipher::Aes128GcmSha256 => hkdf_expand_sha256(secret, &info, length),
        Cipher::Aes256GcmSha384 => hkdf_expand_sha384(secret, &info, length),
    }
}

fn hkdf_expand_sha256(secret: &[u8], info: &[u8], length: usize) -> Result<Vec<u8>, ReplayError> {
    let mut previous = Vec::new();
    let mut output = Vec::with_capacity(length);
    for counter in 1..=u8::MAX {
        let mut hmac = <Hmac<Sha256> as Mac>::new_from_slice(secret).map_err(|_| ReplayError::UnsupportedTls)?;
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
    Err(ReplayError::UnsupportedTls)
}

fn hkdf_expand_sha384(secret: &[u8], info: &[u8], length: usize) -> Result<Vec<u8>, ReplayError> {
    let mut previous = Vec::new();
    let mut output = Vec::with_capacity(length);
    for counter in 1..=u8::MAX {
        let mut hmac = <Hmac<Sha384> as Mac>::new_from_slice(secret).map_err(|_| ReplayError::UnsupportedTls)?;
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
    Err(ReplayError::UnsupportedTls)
}

fn decrypt_tls13_record(
    cipher: Cipher,
    key: &TrafficKey,
    sequence: u64,
    content_type: u8,
    version: u16,
    body: &[u8],
) -> Result<Vec<u8>, ReplayError> {
    if version != TLS_VERSION_1_2 || body.len() < 16 {
        return Err(ReplayError::UnsupportedTls);
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
            .map_err(|_| ReplayError::UnsupportedTls)?
            .to_be_bytes(),
    );
    let tag = Tag::from_slice(tag_bytes);
    let mut plaintext = ciphertext.to_vec();
    let result = match cipher {
        Cipher::Aes128GcmSha256 => Aes128Gcm::new_from_slice(&key.key)
            .map_err(|_| ReplayError::UnsupportedTls)?
            .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
        Cipher::Aes256GcmSha384 => Aes256Gcm::new_from_slice(&key.key)
            .map_err(|_| ReplayError::UnsupportedTls)?
            .decrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext, tag),
    };
    result.map_err(|_| ReplayError::TlsAuthentication)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Endpoint, Flow};

    #[test]
    fn decrypts_tls12_application_data() {
        let client_random = [1; 32];
        let master = [2; 48];
        let key_log = format!("CLIENT_RANDOM {} {}", hex(&client_random), hex(&master));
        let client_hello = hello(1, client_random, None);
        let server_hello = hello(2, [3; 32], Some(0x009c));
        let tls = Tls12::from_hello(&client_hello, &server_hello, 0x009c, &key_log).unwrap();
        let client_records = vec![
            (1, handshake_record(1, &client_hello)),
            (2, change_cipher_spec()),
            (3, encrypt_tls12_record(&tls, Direction::Client, 0, b"client")),
        ];
        let server_records = vec![
            (4, handshake_record(2, &server_hello)),
            (5, change_cipher_spec()),
            (6, encrypt_tls12_record(&tls, Direction::Server, 0, b"server")),
        ];
        let capture = Capture {
            flow: Flow {
                client: Endpoint {
                    address: core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST),
                    port: 1,
                },
                server: Endpoint {
                    address: core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST),
                    port: 2,
                },
                client_stream: vec![(1, client_records.into_iter().flat_map(|(_, record)| record).collect())],
                server_stream: vec![(4, server_records.into_iter().flat_map(|(_, record)| record).collect())],
            },
            tls_key_log: TlsKeyLog::new(key_log),
            gateway_alternates: Vec::new(),
        };

        let plaintext = decrypt_tls(&capture).unwrap();

        assert_eq!(plaintext.client, vec![(1, b"client".to_vec())]);
        assert_eq!(plaintext.server, vec![(4, b"server".to_vec())]);
    }

    #[test]
    fn rejects_tls_without_matching_key_log_entry() {
        let client_hello = hello(1, [1; 32], None);
        let server_hello = hello(2, [3; 32], Some(0x009c));

        let result = Tls::from_records(
            &vec![(1, handshake_record(1, &client_hello))],
            &vec![(2, handshake_record(2, &server_hello))],
            "",
        );

        assert!(matches!(result, Err(ReplayError::MissingTlsSecret)));
    }

    #[test]
    fn reports_standard_security_without_tls_records() {
        let capture = Capture {
            flow: Flow {
                client: endpoint(1),
                server: endpoint(2),
                client_stream: vec![(1, x224_connection(0xe0))],
                server_stream: vec![(2, x224_connection(0xd0))],
            },
            tls_key_log: TlsKeyLog::new(String::new()),
            gateway_alternates: Vec::new(),
        };

        assert!(matches!(decrypt_tls(&capture), Err(ReplayError::StandardSecurity)));
    }

    #[test]
    fn ignores_a_truncated_trailing_tls_record() {
        let mut stream = x224_connection(0xe0);
        stream.extend([TLS_CONTENT_HANDSHAKE, 3, 3, 0, 1]);

        assert_eq!(collect_tls_records(&vec![(1, stream)], 0xe0).unwrap(), Some(Vec::new()));
    }

    #[test]
    fn decrypts_tls13_handshake_and_application_records() {
        let client_random = [1; 32];
        let key_log = [
            ("CLIENT_HANDSHAKE_TRAFFIC_SECRET", [2; 32]),
            ("SERVER_HANDSHAKE_TRAFFIC_SECRET", [3; 32]),
            ("CLIENT_TRAFFIC_SECRET_0", [4; 32]),
            ("SERVER_TRAFFIC_SECRET_0", [5; 32]),
        ]
        .into_iter()
        .map(|(label, secret)| format!("{label} {} {}", hex(&client_random), hex(&secret)))
        .collect::<Vec<_>>()
        .join("\n");
        let tls = Tls13::from_hello(
            &hello(1, client_random, None),
            0x1301,
            &key_log,
            &Vec::new(),
            &Vec::new(),
        )
        .unwrap();
        let finished = [20, 0, 0, 0];
        let client = vec![
            (
                1,
                encrypt_tls13_record(
                    tls.cipher,
                    tls.client_handshake.as_ref().expect("handshake key"),
                    0,
                    &finished,
                    TLS_CONTENT_HANDSHAKE,
                ),
            ),
            (
                2,
                encrypt_tls13_record(
                    tls.cipher,
                    &tls.client_application,
                    0,
                    b"client",
                    TLS_CONTENT_APPLICATION_DATA,
                ),
            ),
        ];
        let server = vec![
            (
                3,
                encrypt_tls13_record(
                    tls.cipher,
                    tls.server_handshake.as_ref().expect("handshake key"),
                    0,
                    &finished,
                    TLS_CONTENT_HANDSHAKE,
                ),
            ),
            (
                4,
                encrypt_tls13_record(
                    tls.cipher,
                    &tls.server_application,
                    0,
                    b"server",
                    TLS_CONTENT_APPLICATION_DATA,
                ),
            ),
        ];

        assert_eq!(
            tls.decrypt(Direction::Client, &client).unwrap(),
            vec![(2, b"client".to_vec())]
        );
        assert_eq!(
            tls.decrypt(Direction::Server, &server).unwrap(),
            vec![(4, b"server".to_vec())]
        );
    }

    fn endpoint(port: u16) -> Endpoint {
        Endpoint {
            address: core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST),
            port,
        }
    }

    fn x224_connection(code: u8) -> Vec<u8> {
        vec![3, 0, 0, 11, 6, code, 0, 0, 0, 0, 0]
    }

    fn hello(kind: u8, random: [u8; 32], suite: Option<u16>) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(TLS_VERSION_1_2.to_be_bytes());
        body.extend(random);
        if kind == 2 {
            body.push(0);
            body.extend(suite.unwrap().to_be_bytes());
            body.push(0);
        }
        body
    }

    fn handshake_record(kind: u8, body: &[u8]) -> Vec<u8> {
        let length = u32::try_from(body.len()).unwrap();
        let mut handshake = vec![kind, 0, 0, 0];
        handshake[1..4].copy_from_slice(&length.to_be_bytes()[1..]);
        handshake.extend(body);
        tls_record(TLS_CONTENT_HANDSHAKE, &handshake)
    }

    fn change_cipher_spec() -> Vec<u8> {
        tls_record(TLS_CONTENT_CHANGE_CIPHER_SPEC, &[1])
    }

    fn encrypt_tls12_record(tls: &Tls12, direction: Direction, sequence: u64, plaintext: &[u8]) -> Vec<u8> {
        let (key, fixed_iv) = match direction {
            Direction::Client => (&tls.client_key, tls.client_iv),
            Direction::Server => (&tls.server_key, tls.server_iv),
        };
        let explicit = [7; 8];
        let mut nonce = [0; 12];
        nonce[..4].copy_from_slice(&fixed_iv);
        nonce[4..].copy_from_slice(&explicit);
        let length = u16::try_from(plaintext.len()).unwrap();
        let mut additional_data = Vec::new();
        additional_data.extend(sequence.to_be_bytes());
        additional_data.push(TLS_CONTENT_APPLICATION_DATA);
        additional_data.extend(TLS_VERSION_1_2.to_be_bytes());
        additional_data.extend(length.to_be_bytes());
        let mut encrypted = plaintext.to_vec();
        let tag = Aes128Gcm::new_from_slice(key)
            .unwrap()
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut encrypted)
            .unwrap();
        let mut body = explicit.to_vec();
        body.extend(encrypted);
        body.extend(tag);
        tls_record(TLS_CONTENT_APPLICATION_DATA, &body)
    }

    fn encrypt_tls13_record(
        cipher: Cipher,
        key: &TrafficKey,
        sequence: u64,
        content: &[u8],
        inner_type: u8,
    ) -> Vec<u8> {
        let mut plaintext = content.to_vec();
        plaintext.push(inner_type);
        let mut nonce = key.iv;
        for (byte, sequence_byte) in nonce[4..].iter_mut().zip(sequence.to_be_bytes()) {
            *byte ^= sequence_byte;
        }
        let ciphertext_len = plaintext.len() + 16;
        let mut additional_data = vec![TLS_CONTENT_APPLICATION_DATA];
        additional_data.extend(TLS_VERSION_1_2.to_be_bytes());
        additional_data.extend(u16::try_from(ciphertext_len).unwrap().to_be_bytes());
        let tag = match cipher {
            Cipher::Aes128GcmSha256 => Aes128Gcm::new_from_slice(&key.key)
                .unwrap()
                .encrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext)
                .unwrap(),
            Cipher::Aes256GcmSha384 => Aes256Gcm::new_from_slice(&key.key)
                .unwrap()
                .encrypt_in_place_detached(Nonce::from_slice(&nonce), &additional_data, &mut plaintext)
                .unwrap(),
        };
        plaintext.extend(tag);
        tls_record(TLS_CONTENT_APPLICATION_DATA, &plaintext)
    }

    fn tls_record(content_type: u8, body: &[u8]) -> Vec<u8> {
        let length = u16::try_from(body.len()).unwrap();
        let mut record = vec![content_type];
        record.extend(TLS_VERSION_1_2.to_be_bytes());
        record.extend(length.to_be_bytes());
        record.extend(body);
        record
    }

    #[test]
    fn decrypts_tunneled_gateway_session() {
        let client_random = [1; 32];
        let master = [2; 48];
        let key_log = format!("CLIENT_RANDOM {} {}", hex(&client_random), hex(&master));
        let client_hello = hello(1, client_random, None);
        let server_hello = hello(2, [3; 32], Some(0x009c));
        let tls = Tls12::from_hello(&client_hello, &server_hello, 0x009c, &key_log).unwrap();

        // Tunneled RDP stream: X.224 connection, then TLS handshake and data.
        let mut inner_client = x224_connection(0xe0);
        inner_client.extend(handshake_record(1, &client_hello));
        inner_client.extend(change_cipher_spec());
        inner_client.extend(encrypt_tls12_record(&tls, Direction::Client, 0, b"client"));
        let mut inner_server = x224_connection(0xd0);
        inner_server.extend(handshake_record(2, &server_hello));
        inner_server.extend(change_cipher_spec());
        inner_server.extend(encrypt_tls12_record(&tls, Direction::Server, 0, b"server"));

        // Wrap each stream in MS-TSGU data packets carried by WebSocket frames.
        let mut outer_client = b"RDG_OUT_DATA /remoteDesktopGateway/ HTTP/1.1\r\n\r\n".to_vec();
        outer_client.extend(gateway_frame(&inner_client, Some([5, 6, 7, 8])));
        let mut outer_server = b"HTTP/1.1 101 Switching Protocols\r\n\r\n".to_vec();
        outer_server.extend(gateway_frame(&inner_server, None));
        let outer = Plaintext {
            client: vec![(1, outer_client)],
            server: vec![(2, outer_server)],
        };

        let tunneled = crate::gateway::extract_tunneled_rdp(&outer).unwrap();
        let plaintext = decrypt_tls_streams(&tunneled.client, &tunneled.server, &key_log).unwrap();

        assert_eq!(plaintext.client, vec![(1, b"client".to_vec())]);
        assert_eq!(plaintext.server, vec![(2, b"server".to_vec())]);
    }

    fn gateway_frame(data: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
        let packet_length = u32::try_from(10 + data.len()).unwrap();
        let mut packet = Vec::new();
        packet.extend(0x0Au16.to_le_bytes());
        packet.extend(0u16.to_le_bytes());
        packet.extend(packet_length.to_le_bytes());
        packet.extend(u16::try_from(data.len()).unwrap().to_le_bytes());
        packet.extend(data);

        let mut frame = vec![0x82];
        frame.push((u8::from(mask.is_some()) << 7) | u8::try_from(packet.len()).unwrap());
        match mask {
            Some(mask) => {
                frame.extend(mask);
                frame.extend(packet.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]));
            }
            None => frame.extend(packet),
        }
        frame
    }
}
