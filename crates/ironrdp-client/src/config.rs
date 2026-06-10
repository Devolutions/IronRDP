use core::fmt;
use core::str::FromStr;
use core::time::Duration;
#[cfg(windows)]
use std::path::PathBuf;

use anyhow::Context as _;
use ironrdp::connector;
use ironrdp_mstsgu::GwConnectTarget;
use url::Url;

/// Fully resolved client configuration.
///
/// This is the typed surface consumed by [`crate::rdp::RdpClient`]. Producing a `Config`
/// from CLI arguments, `.rdp` files, or interactive prompts is the consumer's responsibility
/// (see the `ironrdp-viewer` crate for a reference CLI front-end).
#[derive(Clone, Debug)]
pub struct Config {
    pub log_file: Option<String>,
    pub gw: Option<GwConnectTarget>,
    pub kerberos_config: Option<connector::credssp::KerberosConfig>,
    pub destination: Destination,
    pub connector: connector::Config,
    pub clipboard_type: ClipboardType,
    pub rdcleanpath: Option<RDCleanPathConfig>,
    pub fake_events_interval: Option<Duration>,

    /// DVC channel <-> named pipe proxy configuration.
    ///
    /// Each configured proxy enables IronRDP to connect to DVC channel and create a named pipe
    /// server, which will be used for proxying DVC messages to/from user-defined DVC logic
    /// implemented as named pipe clients (either in the same process or in a different process).
    pub dvc_pipe_proxies: Vec<DvcProxyInfo>,

    /// Paths to DVC client plugin DLLs to load (Windows only).
    ///
    /// Each DLL is loaded via `LoadLibraryW` and its `VirtualChannelGetInstance` export is called
    /// to obtain DVC plugin COM objects. Example: `C:\Windows\System32\webauthn.dll`.
    #[cfg(windows)]
    pub dvc_plugins: Vec<PathBuf>,
}

/// Resolved clipboard backend selection.
///
/// Platform-specific details (e.g., which native clipboard backend to use) are handled
/// internally by the library when `Enable` is selected.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClipboardType {
    /// Enable clipboard redirection (use the best available backend).
    Enable,
    /// Disable clipboard redirection entirely.
    Disable,
    /// Use a stub clipboard backend (for testing or headless usage).
    Stub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    name: String,
    port: u16,
}

impl Destination {
    pub fn new(addr: impl Into<String>) -> anyhow::Result<Self> {
        const RDP_DEFAULT_PORT: u16 = 3389;

        let addr = addr.into();

        if let Some(addr_split) = addr.rsplit_once(':') {
            if let Ok(sock_addr) = addr.parse::<core::net::SocketAddr>() {
                Ok(Self {
                    name: sock_addr.ip().to_string(),
                    port: sock_addr.port(),
                })
            } else if addr.parse::<core::net::Ipv6Addr>().is_ok() {
                Ok(Self {
                    name: addr,
                    port: RDP_DEFAULT_PORT,
                })
            } else {
                Ok(Self {
                    name: addr_split.0.to_owned(),
                    port: addr_split.1.parse().context("invalid port")?,
                })
            }
        } else {
            Ok(Self {
                name: addr,
                port: RDP_DEFAULT_PORT,
            })
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Construct a `Destination` from already-validated components.
    ///
    /// Intended for front-ends that have already resolved the host and port from their own
    /// configuration sources (CLI flags, `.rdp` files, IPC schemas).
    pub fn from_parts(name: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            port,
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // IPv6 addresses must be bracketed in host:port notation.
        if self.name.parse::<core::net::Ipv6Addr>().is_ok() {
            write!(f, "[{}]:{}", self.name, self.port)
        } else {
            write!(f, "{}:{}", self.name, self.port)
        }
    }
}

impl FromStr for Destination {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl From<Destination> for connector::ServerName {
    fn from(value: Destination) -> Self {
        Self::new(value.name)
    }
}

impl From<&Destination> for connector::ServerName {
    fn from(value: &Destination) -> Self {
        Self::new(&value.name)
    }
}

#[derive(Clone, Debug)]
pub struct RDCleanPathConfig {
    pub url: Url,
    pub auth_token: String,
}

#[derive(Clone, Debug)]
pub struct DvcProxyInfo {
    pub channel_name: String,
    pub pipe_name: String,
}

impl FromStr for DvcProxyInfo {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('=');
        let channel_name = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing DVC channel name"))?
            .to_owned();
        let pipe_name = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing DVC proxy pipe name"))?
            .to_owned();

        Ok(Self {
            channel_name,
            pipe_name,
        })
    }
}
