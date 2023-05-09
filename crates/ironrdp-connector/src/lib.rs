#[macro_use]
extern crate tracing;

pub mod legacy;

mod channel_connection;
mod connection;
mod connection_finalization;
mod license_exchange;
mod server_name;

use core::any::Any;
use core::fmt;

use ironrdp_pdu::rdp::capability_sets;
use ironrdp_pdu::{gcc, nego, PduHint};

type StaticChannels = std::collections::HashMap<String, u16>;

pub use channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
pub use connection::{ClientConnector, ClientConnectorState, ConnectionResult};
pub use connection_finalization::{ConnectionFinalizationSequence, ConnectionFinalizationState};
pub use license_exchange::{LicenseExchangeSequence, LicenseExchangeState};
pub use server_name::ServerName;
pub use sspi;

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
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Config {
    pub desktop_size: DesktopSize,
    pub security_protocol: nego::SecurityProtocol,
    pub username: String,
    pub password: String,
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
}

pub trait State: Send + Sync + core::fmt::Debug {
    fn name(&self) -> &'static str;
    fn is_terminal(&self) -> bool;
    fn as_any(&self) -> &dyn Any;
}

ironrdp_pdu::assert_obj_safe!(State);

pub fn state_downcast<T: State + Any>(state: &dyn State) -> Option<&T> {
    state.as_any().downcast_ref()
}

pub fn state_is<T: State + Any>(state: &dyn State) -> bool {
    state.as_any().is::<T>()
}

impl State for () {
    fn name(&self) -> &'static str {
        "()"
    }

    fn is_terminal(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn core::any::Any {
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
    pub fn from_size(value: usize) -> Result<Self> {
        core::num::NonZeroUsize::new(value)
            .map(Self::Size)
            .ok_or(Error::new("invalid written length (can’t be zero)"))
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

pub trait Sequence: Send + Sync {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint>;

    fn state(&self) -> &dyn State;

    fn step(&mut self, input: &[u8], output: &mut Vec<u8>) -> Result<Written>;

    fn step_no_input(&mut self, output: &mut Vec<u8>) -> Result<Written> {
        self.step(&[], output)
    }
}

ironrdp_pdu::assert_obj_safe!(Sequence);

pub type Result<T> = std::result::Result<T, Error>;

#[non_exhaustive]
#[derive(Debug)]
pub enum ErrorKind {
    Pdu(ironrdp_pdu::Error),
    Credssp(sspi::Error),
    AccessDenied,
    Custom(Box<dyn std::error::Error + Sync + Send + 'static>),
    General,
}

#[derive(Debug)]
pub struct Error {
    pub context: &'static str,
    pub kind: ErrorKind,
    pub reason: Option<String>,
}

impl Error {
    pub fn new(context: &'static str) -> Self {
        Self {
            context,
            kind: ErrorKind::General,
            reason: None,
        }
    }

    pub fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_custom<E>(mut self, custom_error: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        self.kind = ErrorKind::Custom(Box::new(custom_error));
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Pdu(e) => Some(e),
            ErrorKind::Credssp(e) => Some(e),
            ErrorKind::AccessDenied => None,
            ErrorKind::Custom(e) => Some(e.as_ref()),
            ErrorKind::General => None,
        }
    }
}

impl From<Error> for std::io::Error {
    fn from(error: Error) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, error)
    }
}

impl From<ironrdp_pdu::Error> for Error {
    fn from(value: ironrdp_pdu::Error) -> Self {
        Self {
            context: "invalid payload",
            kind: ErrorKind::Pdu(value),
            reason: None,
        }
    }
}

impl From<sspi::Error> for Error {
    fn from(value: sspi::Error) -> Self {
        Self {
            context: "CredSSP",
            kind: ErrorKind::Credssp(value),
            reason: None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.context)?;

        match &self.kind {
            ErrorKind::Pdu(e) => {
                if f.alternate() {
                    write!(f, ": {e}")?;
                }
            }
            ErrorKind::Credssp(e) => {
                if f.alternate() {
                    write!(f, ": {e}")?;
                }
            }
            ErrorKind::AccessDenied => {
                write!(f, ": access denied")?;
            }
            ErrorKind::Custom(e) => {
                if f.alternate() {
                    write!(f, ": {e}")?;

                    let mut next_source = e.source();
                    while let Some(e) = next_source {
                        write!(f, ", caused by: {e}")?;
                        next_source = e.source();
                    }
                }
            }
            ErrorKind::General => {}
        }

        if let Some(reason) = &self.reason {
            write!(f, " ({reason})")?;
        }

        Ok(())
    }
}

pub trait ConnectorResultExt {
    fn with_context(self, context: &'static str) -> Self;
    fn with_kind(self, kind: ErrorKind) -> Self;
    fn with_custom<E>(self, custom_error: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
    fn with_reason(self, reason: impl Into<String>) -> Self;
}

impl<T> ConnectorResultExt for Result<T> {
    fn with_context(self, context: &'static str) -> Self {
        self.map_err(|mut e| {
            e.context = context;
            e
        })
    }

    fn with_kind(self, kind: ErrorKind) -> Self {
        self.map_err(|mut e| {
            e.kind = kind;
            e
        })
    }

    fn with_custom<E>(self, custom_error: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        self.map_err(|mut e| {
            e.kind = ErrorKind::Custom(Box::new(custom_error));
            e
        })
    }

    fn with_reason(self, reason: impl Into<String>) -> Self {
        self.map_err(|mut e| {
            e.reason = Some(reason.into());
            e
        })
    }
}
