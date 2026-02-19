use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const CONTROL_PIPE_ENV: &str = "IRONRDP_WTS_CONTROL_PIPE";
pub const DEFAULT_CONTROL_PIPE_NAME: &str = "IronRdpWtsControl";
pub const DEFAULT_MAX_FRAME_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCommand {
    StartListen { listener_name: String },
    StopListen { listener_name: String },
    WaitForIncoming { listener_name: String, timeout_ms: u32 },
    AcceptConnection { connection_id: u32 },
    CloseConnection { connection_id: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceEvent {
    Ack,
    ListenerStarted { listener_name: String },
    ListenerStopped { listener_name: String },
    IncomingConnection {
        listener_name: String,
        connection_id: u32,
        peer_addr: Option<String>,
    },
    NoIncoming,
    ConnectionReady { connection_id: u32 },
    ConnectionBroken { connection_id: u32, reason: String },
    Error { message: String },
}

pub fn resolve_pipe_name_from_env() -> Option<String> {
    std::env::var(CONTROL_PIPE_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn default_pipe_name() -> String {
    DEFAULT_CONTROL_PIPE_NAME.to_owned()
}

pub fn pipe_path(pipe_name: &str) -> String {
    if pipe_name.starts_with(r"\\.\pipe\") {
        pipe_name.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe_name}")
    }
}

pub fn write_frame(writer: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;

    writer.write_all(&payload_len.to_le_bytes())?;
    writer.write_all(payload)
}

pub fn read_frame(reader: &mut impl Read, max_frame_size: usize) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;

    let frame_len_u32 = u32::from_le_bytes(len_buf);
    let frame_len = usize::try_from(frame_len_u32)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length does not fit in usize"))?;

    if frame_len > max_frame_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length exceeds maximum",
        ));
    }

    let mut payload = vec![0u8; frame_len];
    reader.read_exact(&mut payload)?;

    Ok(payload)
}

pub fn write_json_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("failed to serialize message: {error}")))?;

    write_frame(writer, &payload)
}

pub fn read_json_message<T: DeserializeOwned>(reader: &mut impl Read, max_frame_size: usize) -> io::Result<T> {
    let payload = read_frame(reader, max_frame_size)?;

    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("failed to decode message: {error}")))
}
