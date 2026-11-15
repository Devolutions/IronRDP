use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

pub mod display;

pub const CONTROL_PIPE_ENV: &str = "IRONRDP_WTS_CONTROL_PIPE";
pub const DEFAULT_CONTROL_PIPE_NAME: &str = "IronRdpWtsControl";
pub const DEFAULT_MAX_FRAME_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCommand {
    StartListen {
        listener_name: String,
        #[serde(default)]
        port: Option<u16>,
    },
    StopListen {
        listener_name: String,
    },
    WaitForIncoming {
        listener_name: String,
        timeout_ms: u32,
        #[serde(default)]
        port: Option<u16>,
    },
    AcceptConnection {
        connection_id: u32,
    },
    CloseConnection {
        connection_id: u32,
    },
    /// Retrieve the CredSSP-derived plaintext credentials for a connection.
    ///
    /// The provider DLL calls this after `AcceptConnection` and before `GetUserCredentials` is
    /// invoked by RDS, so that the companion service can relay the credentials it captured during
    /// the NLA handshake back to the provider.
    GetConnectionCredentials {
        connection_id: u32,
    },
    GetRemoteAppSettings {
        connection_id: u32,
    },
    /// Notify the companion service of the session ID for a connection.
    ///
    /// The provider DLL calls this when WTS invokes `NotifySessionId`, so the companion can
    /// target the correct session for desktop capture instead of guessing via console/enumeration.
    SetCaptureSessionId {
        connection_id: u32,
        session_id: u32,
    },
    /// Notify the companion service that the WDDM IDD driver has been loaded for a session.
    ///
    /// The companion service consumes this signal to correlate provider session assignment with
    /// IDD readiness diagnostics. The companion service does not receive a usable driver handle
    /// (handles are process-local).
    NotifyIddDriverLoaded {
        session_id: u32,
    },
    OpenVirtualChannel {
        connection_id: u32,
        session_id: u32,
        endpoint_name: String,
        /// Whether the endpoint is a static virtual channel.
        ///
        /// A frame from a provider that predates the field opens a dynamic channel.
        #[serde(default)]
        static_channel: bool,
    },
}

impl ProviderCommand {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::StartListen { .. } => "start_listen",
            Self::StopListen { .. } => "stop_listen",
            Self::WaitForIncoming { .. } => "wait_for_incoming",
            Self::AcceptConnection { .. } => "accept_connection",
            Self::CloseConnection { .. } => "close_connection",
            Self::GetConnectionCredentials { .. } => "get_connection_credentials",
            Self::GetRemoteAppSettings { .. } => "get_remote_app_settings",
            Self::SetCaptureSessionId { .. } => "set_capture_session_id",
            Self::NotifyIddDriverLoaded { .. } => "notify_idd_driver_loaded",
            Self::OpenVirtualChannel { .. } => "open_virtual_channel",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceEvent {
    Ack,
    ListenerStarted {
        listener_name: String,
    },
    ListenerStopped {
        listener_name: String,
    },
    IncomingConnection {
        listener_name: String,
        connection_id: u32,
        peer_addr: Option<String>,
    },
    NoIncoming,
    ConnectionReady {
        connection_id: u32,
    },
    RemoteAppSettings {
        connection_id: u32,
        settings: Option<RemoteAppSettings>,
    },
    ConnectionBroken {
        connection_id: u32,
        reason: String,
    },
    /// Plaintext credentials obtained from the CredSSP/NLA handshake for a connection.
    ///
    /// Returned in response to [`ProviderCommand::GetConnectionCredentials`].  The provider DLL
    /// stores these and returns them from `IWRdsProtocolConnection::GetUserCredentials` so that
    /// Winlogon can create a real interactive session.
    ConnectionCredentials {
        connection_id: u32,
        username: String,
        domain: String,
        password: String,
    },
    /// No CredSSP credentials are available for the requested connection.
    ///
    /// The companion service returns this when it has not yet completed the NLA handshake or when
    /// the connection is not known.  The provider should fall back to showing the logon dialog.
    NoCredentials {
        connection_id: u32,
    },
    VirtualChannelReady {
        pipe_name: String,
    },
    Error {
        message: String,
    },
}

impl ServiceEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ack => "ack",
            Self::ListenerStarted { .. } => "listener_started",
            Self::ListenerStopped { .. } => "listener_stopped",
            Self::IncomingConnection { .. } => "incoming_connection",
            Self::NoIncoming => "no_incoming",
            Self::ConnectionReady { .. } => "connection_ready",
            Self::RemoteAppSettings { .. } => "remote_app_settings",
            Self::ConnectionBroken { .. } => "connection_broken",
            Self::ConnectionCredentials { .. } => "connection_credentials",
            Self::NoCredentials { .. } => "no_credentials",
            Self::VirtualChannelReady { .. } => "virtual_channel_ready",
            Self::Error { .. } => "error",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAppSettings {
    pub enabled: bool,
    pub client_rail_support_level: Option<u32>,
    pub window_capability_set: Option<Vec<u8>>,
    #[serde(default)]
    pub client_info_flags: u32,
    #[serde(default)]
    pub performance_flags: u32,
    #[serde(default)]
    pub keyboard_layout: u32,
    #[serde(default)]
    pub keyboard_type: u32,
    #[serde(default)]
    pub keyboard_subtype: u32,
    #[serde(default)]
    pub keyboard_function_key: u32,
    #[serde(default)]
    pub ime_file_name: String,
    #[serde(default)]
    pub desktop_width: u16,
    #[serde(default)]
    pub desktop_height: u16,
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
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;

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

pub async fn read_async_frame(
    reader: &mut (dyn AsyncRead + Unpin + Send),
    max_frame_size: usize,
) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;

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
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

pub async fn write_async_frame(
    writer: &mut (dyn AsyncWrite + Unpin + Send),
    payload: &[u8],
    max_frame_size: usize,
) -> io::Result<()> {
    if payload.len() > max_frame_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame length exceeds maximum",
        ));
    }

    let payload_len =
        u32::try_from(payload.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;

    writer.write_all(&payload_len.to_le_bytes()).await?;
    writer.write_all(payload).await
}

pub fn write_json_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(message).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize message: {error}"),
        )
    })?;

    write_frame(writer, &payload)
}

pub fn read_json_message<T: DeserializeOwned>(reader: &mut impl Read, max_frame_size: usize) -> io::Result<T> {
    let payload = read_frame(reader, max_frame_size)?;

    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("failed to decode message: {error}")))
}
