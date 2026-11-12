//! Test-only stand-ins for the client's camera, microphone and authenticator.
//!
//! The camera shows solid green, the microphone plays a 440 Hz tone, and the authenticator
//! registers credentials with "none" attestation and never finds existing ones.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};

use crate::remote_devices::{AuthenticatorEvent, PixelFormat, RemoteAuthenticator, RemoteMedia};

const SAMPLE_RATE: u32 = 48_000;
const TONE_HZ: f32 = 440.0;
const CHUNK: Duration = Duration::from_millis(10);
/// BT.601 limited-range green.
const GREEN: [u8; 3] = [145, 54, 34];
const AAGUID: [u8; 16] = *b"ironrdp-cef-test";
/// The P-256 generator point, a valid public key with no matching private key in use anywhere.
const GENERATOR_X: [u8; 32] = [
    0x6B, 0x17, 0xD1, 0xF2, 0xE1, 0x2C, 0x42, 0x47, 0xF8, 0xBC, 0xE6, 0xE5, 0x63, 0xA4, 0x40, 0xF2, 0x77, 0x03, 0x7D,
    0x81, 0x2D, 0xEB, 0x33, 0xA0, 0xF4, 0xA1, 0x39, 0x45, 0xD8, 0x98, 0xC2, 0x96,
];
const GENERATOR_Y: [u8; 32] = [
    0x4F, 0xE3, 0x42, 0xE2, 0xFE, 0x1A, 0x7F, 0x9B, 0x8E, 0xE7, 0xEB, 0x4A, 0x7C, 0x0F, 0x9E, 0x16, 0x2B, 0xCE, 0x33,
    0x57, 0x6B, 0x31, 0x5E, 0xCE, 0xCB, 0xB6, 0x40, 0x68, 0x37, 0xBF, 0x51, 0xF5,
];

pub struct DeviceSimulator {
    stop: Arc<AtomicBool>,
    media: Option<JoinHandle<()>>,
}

impl DeviceSimulator {
    /// Starts feeding media and answers authenticator requests; call on the CEF UI thread.
    pub fn start(media: RemoteMedia, authenticator: &mut RemoteAuthenticator) -> anyhow::Result<Self> {
        let responder = authenticator.responder();
        authenticator.set_handler(move |event| {
            if let AuthenticatorEvent::Request { id, message } = event {
                let response = respond(&message);
                // Answering off the UI thread keeps the reply out of the request's dispatch.
                std::thread::spawn(move || {
                    if let Err(error) = responder.respond(id, &response) {
                        tracing::warn!(%error, id, "Simulated authenticator response failed");
                    }
                });
            }
        })?;
        authenticator.set_platform_available(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let media = std::thread::Builder::new().name("device-simulator".to_owned()).spawn({
            let stop = Arc::clone(&stop);
            move || feed_media(media, &stop)
        })?;
        Ok(Self {
            stop,
            media: Some(media),
        })
    }
}

impl Drop for DeviceSimulator {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(media) = self.media.take() {
            let _ = media.join();
        }
    }
}

fn feed_media(media: RemoteMedia, stop: &AtomicBool) {
    let start = Instant::now();
    let mut next_frame = start;
    let mut phase = 0.0f32;
    let mut tone = vec![0i16; usize::try_from(SAMPLE_RATE).unwrap_or(48_000) / 100];
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(CHUNK);
        if 0 < media.microphone_streams() {
            for sample in &mut tone {
                *sample = (phase.sin() * f32::from(i16::MAX / 2)) as i16;
                phase = (phase + std::f32::consts::TAU * TONE_HZ / SAMPLE_RATE as f32) % std::f32::consts::TAU;
            }
            if let Err(error) = media.push_microphone(&tone, 1, SAMPLE_RATE) {
                tracing::warn!(%error, "Simulated microphone push failed");
            }
        }
        let Some(format) = media.camera_format() else {
            continue;
        };
        let now = Instant::now();
        if now < next_frame {
            continue;
        }
        next_frame = now + Duration::from_secs_f32(1.0 / format.frame_rate.clamp(1.0, 60.0));
        let (width, height) = (format.width.max(2), format.height.max(2));
        let luma = (width * height) as usize;
        let chroma = (width.div_ceil(2) * height.div_ceil(2)) as usize;
        let mut frame = vec![GREEN[0]; luma];
        frame.resize(luma + chroma, GREEN[1]);
        frame.resize(luma + 2 * chroma, GREEN[2]);
        if let Err(error) = media.push_camera_frame(PixelFormat::I420, width, height, &frame, now - start) {
            tracing::warn!(%error, "Simulated camera push failed");
        }
    }
}

const CTAP_OK: u8 = 0x00;
const CTAP_INVALID_COMMAND: u8 = 0x01;
const CTAP_INVALID_CBOR: u8 = 0x12;
const CTAP_NO_CREDENTIALS: u8 = 0x2E;

/// Answers one CTAP2 request: command byte, then CBOR parameters.
fn respond(message: &[u8]) -> Vec<u8> {
    match message.split_first() {
        Some((0x04, _)) => {
            let mut out = vec![CTAP_OK];
            put_head(&mut out, 5, 3);
            put_head(&mut out, 0, 1);
            put_head(&mut out, 4, 1);
            put_text(&mut out, "FIDO_2_0");
            put_head(&mut out, 0, 3);
            put_bytes(&mut out, &AAGUID);
            put_head(&mut out, 0, 4);
            put_head(&mut out, 5, 3);
            for option in ["rk", "up", "uv"] {
                put_text(&mut out, option);
                out.push(0xF5);
            }
            out
        }
        Some((0x01, parameters)) => match relying_party(parameters) {
            Some(rp_id) => make_credential(rp_id),
            None => vec![CTAP_INVALID_CBOR],
        },
        Some((0x02, _)) => vec![CTAP_NO_CREDENTIALS],
        _ => vec![CTAP_INVALID_COMMAND],
    }
}

fn make_credential(rp_id: &str) -> Vec<u8> {
    let credential_id = &Sha256::digest(rp_id.as_bytes())[..16];
    let mut auth_data = Sha256::digest(rp_id.as_bytes()).to_vec();
    // User present, user verified, attested credential data included; sign count zero.
    auth_data.extend_from_slice(&[0x45, 0, 0, 0, 0]);
    auth_data.extend_from_slice(&AAGUID);
    auth_data.extend_from_slice(&16u16.to_be_bytes());
    auth_data.extend_from_slice(credential_id);
    // COSE_Key: kty EC2, alg ES256, crv P-256, x, y.
    put_head(&mut auth_data, 5, 5);
    put_head(&mut auth_data, 0, 1);
    put_head(&mut auth_data, 0, 2);
    put_head(&mut auth_data, 0, 3);
    put_head(&mut auth_data, 1, 6);
    put_head(&mut auth_data, 1, 0);
    put_head(&mut auth_data, 0, 1);
    put_head(&mut auth_data, 1, 1);
    put_bytes(&mut auth_data, &GENERATOR_X);
    put_head(&mut auth_data, 1, 2);
    put_bytes(&mut auth_data, &GENERATOR_Y);

    let mut out = vec![CTAP_OK];
    put_head(&mut out, 5, 3);
    put_head(&mut out, 0, 1);
    put_text(&mut out, "none");
    put_head(&mut out, 0, 2);
    put_bytes(&mut out, &auth_data);
    put_head(&mut out, 0, 3);
    put_head(&mut out, 5, 0);
    out
}

/// Returns the `rp.id` of a MakeCredential parameter map.
fn relying_party(cbor: &[u8]) -> Option<&str> {
    let mut at = 0;
    let (5, entries) = head(cbor, &mut at)? else {
        return None;
    };
    for _ in 0..entries {
        let key = head(cbor, &mut at)?;
        if key != (0, 2) {
            skip(cbor, &mut at)?;
            continue;
        }
        let (5, fields) = head(cbor, &mut at)? else {
            return None;
        };
        for _ in 0..fields {
            let name = text(cbor, &mut at)?;
            if name == "id" {
                return text(cbor, &mut at);
            }
            skip(cbor, &mut at)?;
        }
        return None;
    }
    None
}

fn head(cbor: &[u8], at: &mut usize) -> Option<(u8, u64)> {
    let initial = *cbor.get(*at)?;
    *at += 1;
    let width = match initial & 0x1F {
        info @ 0..=23 => return Some((initial >> 5, u64::from(info))),
        24 => 1,
        25 => 2,
        26 => 4,
        27 => 8,
        _ => return None,
    };
    let bytes = cbor.get(*at..*at + width)?;
    *at += width;
    Some((
        initial >> 5,
        bytes.iter().fold(0, |value, byte| value << 8 | u64::from(*byte)),
    ))
}

fn skip(cbor: &[u8], at: &mut usize) -> Option<()> {
    let (major, argument) = head(cbor, at)?;
    match major {
        2 | 3 => {
            *at = at.checked_add(usize::try_from(argument).ok()?)?;
            (*at <= cbor.len()).then_some(())
        }
        4 | 5 => {
            for _ in 0..argument * u64::from(major - 3) {
                skip(cbor, at)?;
            }
            Some(())
        }
        6 => skip(cbor, at),
        _ => Some(()),
    }
}

fn text<'a>(cbor: &'a [u8], at: &mut usize) -> Option<&'a str> {
    let (3, length) = head(cbor, at)? else {
        return None;
    };
    let end = at.checked_add(usize::try_from(length).ok()?)?;
    let value = std::str::from_utf8(cbor.get(*at..end)?).ok()?;
    *at = end;
    Some(value)
}

fn put_head(out: &mut Vec<u8>, major: u8, argument: u64) {
    let major = major << 5;
    match argument {
        0..=23 => out.push(major | argument as u8),
        24..=0xFF => out.extend_from_slice(&[major | 24, argument as u8]),
        0x100..=0xFFFF => {
            out.push(major | 25);
            out.extend_from_slice(&(argument as u16).to_be_bytes());
        }
        _ => {
            out.push(major | 26);
            out.extend_from_slice(&(argument as u32).to_be_bytes());
        }
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_head(out, 2, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    put_head(out, 3, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
}
