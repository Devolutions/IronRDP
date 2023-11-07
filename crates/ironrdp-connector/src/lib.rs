#[cfg(windows)]
extern crate winapi;

#[macro_use]
extern crate tracing;

#[macro_use]
mod macros;

pub mod legacy;

mod channel_connection;
mod connection;
mod connection_finalization;
pub mod credssp_sequence;
mod license_exchange;
mod server_name;

use core::any::Any;
use core::fmt;

pub use channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
pub use connection::{ClientConnector, ClientConnectorState, ConnectionResult};
pub use connection_finalization::{ConnectionFinalizationSequence, ConnectionFinalizationState};
use ironrdp_pdu::rdp::capability_sets;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{gcc, nego, PduHint};
pub use license_exchange::{LicenseExchangeSequence, LicenseExchangeState};
pub use server_name::ServerName;
pub use sspi;
use sspi::KerberosConfig as SSPIKerberosConfig;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct GraphicsConfig {
    pub avc444: bool,
    pub h264: bool,
    pub thin_client: bool,
    pub small_cache: bool,
    pub capabilities: u32,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct BitmapConfig {
    pub lossy_compression: bool,
    pub color_depth: u32,
}

#[derive(Debug, Clone)]
pub enum Credentials {
    UsernamePassword { username: String, password: String },
    SmartCard { pin: String },
}

impl Credentials {
    fn username(&self) -> &str {
        match self {
            Self::UsernamePassword { username, .. } => username,
            Self::SmartCard { .. } => "", // Username is ultimately provided by the smart card certificate.
        }
    }

    fn secret(&self) -> &str {
        match self {
            Self::UsernamePassword { password, .. } => password,
            Self::SmartCard { pin, .. } => pin,
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Config {
    pub desktop_size: DesktopSize,
    pub security_protocol: nego::SecurityProtocol,
    pub credentials: Credentials,
    pub domain: Option<String>,
    /// The build number of the client.
    pub client_build: u32,
    /// Name of the client computer. Truncated to the 15 first characters.
    pub client_name: String,
    pub keyboard_type: gcc::KeyboardType,
    pub keyboard_subtype: u32,
    pub keyboard_functional_keys_count: u32,
    pub ime_file_name: String,
    pub graphics: Option<GraphicsConfig>,
    pub bitmap: Option<BitmapConfig>,
    pub dig_product_id: String,
    pub client_dir: String,
    pub platform: capability_sets::MajorPlatformType,
    pub no_server_pointer: bool,
    /// If true, the INFO_AUTOLOGON flag is set in the [`ironrdp_pdu::rdp::ClientInfoPdu`].
    pub autologon: bool,
}

#[derive(Debug, Clone, Default)]
pub struct KerberosConfig {
    pub kdc_proxy_url: Option<url::Url>,
    pub hostname: Option<String>,
}

impl KerberosConfig {
    pub fn new(kdc_proxy_url: Option<String>, hostname: Option<String>) -> ConnectorResult<Self> {
        let kdc_proxy_url = kdc_proxy_url
            .map(|url| url::Url::parse(&url))
            .transpose()
            .map_err(|e| custom_err!("invalid KDC URL", e))?;
        Ok(Self {
            kdc_proxy_url,
            hostname,
        })
    }
}

impl From<KerberosConfig> for SSPIKerberosConfig {
    fn from(val: KerberosConfig) -> Self {
        SSPIKerberosConfig {
            url: val.kdc_proxy_url,
            hostname: val.hostname,
        }
    }
}

ironrdp_pdu::assert_impl!(Config: Send, Sync);

pub trait State: Send + core::fmt::Debug + 'static {
    fn name(&self) -> &'static str;
    fn is_terminal(&self) -> bool;
    fn as_any(&self) -> &dyn Any;
}

ironrdp_pdu::assert_obj_safe!(State);

pub fn state_downcast<T: State>(state: &dyn State) -> Option<&T> {
    state.as_any().downcast_ref()
}

pub fn state_is<T: State>(state: &dyn State) -> bool {
    state.as_any().is::<T>()
}

impl State for () {
    fn name(&self) -> &'static str {
        "()"
    }

    fn is_terminal(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Written {
    Nothing,
    Size(core::num::NonZeroUsize),
}

impl Written {
    #[inline]
    pub fn from_size(value: usize) -> ConnectorResult<Self> {
        core::num::NonZeroUsize::new(value)
            .map(Self::Size)
            .ok_or_else(|| ConnectorError::general("invalid written length (can’t be zero)"))
    }

    #[inline]
    pub fn is_nothing(self) -> bool {
        matches!(self, Self::Nothing)
    }

    #[inline]
    pub fn size(self) -> Option<usize> {
        if let Self::Size(size) = self {
            Some(size.get())
        } else {
            None
        }
    }
}

pub trait Sequence: Send {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint>;

    fn state(&self) -> &dyn State;

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written>;

    fn step_no_input(&mut self, output: &mut WriteBuf) -> ConnectorResult<Written> {
        self.step(&[], output)
    }
}

ironrdp_pdu::assert_obj_safe!(Sequence);

pub type ConnectorResult<T> = Result<T, ConnectorError>;

#[non_exhaustive]
#[derive(Debug)]
pub enum ConnectorErrorKind {
    Pdu(ironrdp_pdu::PduError),
    Credssp(sspi::Error),
    Reason(String),
    AccessDenied,
    General,
    Custom,
}

impl fmt::Display for ConnectorErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            ConnectorErrorKind::Pdu(_) => write!(f, "PDU error"),
            ConnectorErrorKind::Credssp(_) => write!(f, "CredSSP"),
            ConnectorErrorKind::Reason(description) => write!(f, "reason: {description}"),
            ConnectorErrorKind::AccessDenied => write!(f, "access denied"),
            ConnectorErrorKind::General => write!(f, "general error"),
            ConnectorErrorKind::Custom => write!(f, "custom error"),
        }
    }
}

impl std::error::Error for ConnectorErrorKind {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self {
            ConnectorErrorKind::Pdu(e) => Some(e),
            ConnectorErrorKind::Credssp(e) => Some(e),
            ConnectorErrorKind::Reason(_) => None,
            ConnectorErrorKind::AccessDenied => None,
            ConnectorErrorKind::Custom => None,
            ConnectorErrorKind::General => None,
        }
    }
}

pub type ConnectorError = ironrdp_error::Error<ConnectorErrorKind>;

pub trait ConnectorErrorExt {
    fn pdu(error: ironrdp_pdu::PduError) -> Self;
    fn general(context: &'static str) -> Self;
    fn reason(context: &'static str, reason: impl Into<String>) -> Self;
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
}

impl ConnectorErrorExt for ConnectorError {
    fn pdu(error: ironrdp_pdu::PduError) -> Self {
        Self::new("invalid payload", ConnectorErrorKind::Pdu(error))
    }

    fn general(context: &'static str) -> Self {
        Self::new(context, ConnectorErrorKind::General)
    }

    fn reason(context: &'static str, reason: impl Into<String>) -> Self {
        Self::new(context, ConnectorErrorKind::Reason(reason.into()))
    }

    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        Self::new(context, ConnectorErrorKind::Custom).with_source(e)
    }
}

pub trait ConnectorResultExt {
    #[must_use]
    fn with_context(self, context: &'static str) -> Self;
    #[must_use]
    fn with_source<E>(self, source: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
}

impl<T> ConnectorResultExt for ConnectorResult<T> {
    fn with_context(self, context: &'static str) -> Self {
        self.map_err(|mut e| {
            e.context = context;
            e
        })
    }

    fn with_source<E>(self, source: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        self.map_err(|e| e.with_source(source))
    }
}
