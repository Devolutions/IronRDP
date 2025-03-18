#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
extern crate tracing;

#[macro_use]
mod macros;

pub mod legacy;

mod channel_connection;
mod connection;
pub mod connection_activation;
mod connection_finalization;
pub mod credssp;
mod license_exchange;
mod server_name;

use core::any::Any;
use core::fmt;
use std::sync::Arc;

use ironrdp_core::{encode_buf, encode_vec, Encode, WriteBuf};
use ironrdp_pdu::nego::NegoRequestData;
use ironrdp_pdu::rdp::capability_sets::{self, BitmapCodecs};
use ironrdp_pdu::rdp::client_info::PerformanceFlags;
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{gcc, x224, PduHint};
pub use sspi;

pub use self::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
pub use self::connection::{encode_send_data_request, ClientConnector, ClientConnectorState, ConnectionResult};
pub use self::connection_finalization::{ConnectionFinalizationSequence, ConnectionFinalizationState};
pub use self::license_exchange::{LicenseExchangeSequence, LicenseExchangeState};
pub use self::server_name::ServerName;
pub use crate::license_exchange::LicenseCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct BitmapConfig {
    pub lossy_compression: bool,
    pub color_depth: u32,
    pub codecs: BitmapCodecs,
}

#[derive(Debug, Clone)]
pub struct SmartCardIdentity {
    /// DER-encoded X509 certificate
    pub certificate: Vec<u8>,
    /// Smart card reader name
    pub reader_name: String,
    /// Smart card key container name
    pub container_name: String,
    /// Smart card CSP name
    pub csp_name: String,
    /// DER-encoded RSA 2048-bit private key
    pub private_key: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum Credentials {
    UsernamePassword {
        username: String,
        password: String,
    },
    SmartCard {
        pin: String,
        config: Option<SmartCardIdentity>,
    },
}

impl Credentials {
    fn username(&self) -> Option<&str> {
        match self {
            Self::UsernamePassword { username, .. } => Some(username),
            Self::SmartCard { .. } => None, // Username is ultimately provided by the smart card certificate.
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
    /// The initial desktop size to request
    pub desktop_size: DesktopSize,
    /// The initial desktop scale factor to request.
    ///
    /// This becomes the `desktop_scale_factor` in the [`TS_UD_CS_CORE`](gcc::ClientCoreOptionalData) structure.
    pub desktop_scale_factor: u32,
    /// TLS + Graphical login (legacy)
    ///
    /// Also called SSL or TLS security protocol.
    /// The PROTOCOL_SSL flag will be set.
    ///
    /// When this security protocol is negotiated, the RDP server will show a graphical login screen.
    /// For Windows, it means that the login subsystem (winlogon.exe) and the GDI graphics subsystem
    /// will be initiated and the user will authenticate himself using LogonUI.exe, as if
    /// using the physical machine directly.
    ///
    /// This security protocol is being phased out because it’s not great security-wise.
    /// Indeed, the whole RDP connection sequence will be performed, allowing anyone to effectively
    /// open a RDP session session with all static channels joined and active (e.g.: I/O, clipboard,
    /// sound, drive redirection, etc). This exposes a wide attack surface with many impacts on both
    /// the client and the server.
    ///
    /// - Man-in-the-middle (MITM)
    /// - Server-side takeover
    /// - Client-side file stealing
    /// - Client-side takeover
    ///
    /// Recommended reads on this topic:
    ///
    /// - <https://www.gosecure.net/blog/2018/12/19/rdp-man-in-the-middle-smile-youre-on-camera/>
    /// - <https://www.gosecure.net/divi_overlay/mitigating-the-risks-of-remote-desktop-protocols/>
    /// - <https://gosecure.github.io/presentations/2021-08-05_blackhat-usa/BlackHat-USA-21-Arsenal-PyRDP-OlivierBilodeau.pdf>
    /// - <https://gosecure.github.io/presentations/2022-10-06_sector/OlivierBilodeau-Purple_RDP.pdf>
    ///
    /// By setting this option to `false`, it’s possible to effectively enforce usage of NLA on client side.
    pub enable_tls: bool,
    /// TLS + Network Level Authentication (NLA) using CredSSP
    ///
    /// The PROTOCOL_HYBRID and PROTOCOL_HYBRID_EX flags will be set.
    ///
    /// NLA is allowing authentication to be performed before session establishment.
    ///
    /// This option includes the extended CredSSP early user authorization result PDU.
    /// This PDU is used by the server to deny access before any credentials (except for the username)
    /// have been submitted, e.g.: typically if the user does not have the necessary remote access
    /// privileges.
    ///
    /// The attack surface is considerably reduced in comparison to the legacy "TLS" security protocol.
    /// For this reason, it is recommended to set `enable_tls` to `false` when connecting to NLA-capable
    /// computers.
    #[doc(alias("enable_nla", "nla"))]
    pub enable_credssp: bool,
    pub credentials: Credentials,
    pub domain: Option<String>,
    /// The build number of the client.
    pub client_build: u32,
    /// Name of the client computer
    ///
    /// The name will be truncated to the 15 first characters.
    pub client_name: String,
    pub keyboard_type: gcc::KeyboardType,
    pub keyboard_subtype: u32,
    pub keyboard_functional_keys_count: u32,
    pub keyboard_layout: u32,
    pub ime_file_name: String,
    pub bitmap: Option<BitmapConfig>,
    pub dig_product_id: String,
    pub client_dir: String,
    pub platform: capability_sets::MajorPlatformType,
    /// Unique identifier for the computer
    ///
    ///  Each 32-bit integer contains client hardware-specific data helping the server uniquely identify the client.
    pub hardware_id: Option<[u32; 4]>,
    /// Optional data for the x224 connection request.
    ///
    /// Fallbacks to a sensible default depending on the provided credentials:
    ///
    /// - A cookie containing the username for a username/password.
    /// - Nothing for a smart card.
    pub request_data: Option<NegoRequestData>,
    /// If true, the INFO_AUTOLOGON flag is set in the [`ClientInfoPdu`](ironrdp_pdu::rdp::ClientInfoPdu)
    pub autologon: bool,
    /// If true, the INFO_NOAUDIOPLAYBACK flag is set in the [`ClientInfoPdu`](ironrdp_pdu::rdp::ClientInfoPdu)
    pub no_audio_playback: bool,

    pub license_cache: Option<Arc<dyn LicenseCache>>,

    // FIXME(@CBenoit): these are client-only options, not part of the connector.
    pub no_server_pointer: bool,
    pub pointer_software_rendering: bool,
    pub performance_flags: PerformanceFlags,
}

ironrdp_core::assert_impl!(Config: Send, Sync);

pub trait State: Send + fmt::Debug + 'static {
    fn name(&self) -> &'static str;
    fn is_terminal(&self) -> bool;
    fn as_any(&self) -> &dyn Any;
}

ironrdp_core::assert_obj_safe!(State);

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

ironrdp_core::assert_obj_safe!(Sequence);

pub type ConnectorResult<T> = Result<T, ConnectorError>;

#[non_exhaustive]
#[derive(Debug)]
pub enum ConnectorErrorKind {
    Encode(ironrdp_core::EncodeError),
    Decode(ironrdp_core::DecodeError),
    Credssp(sspi::Error),
    Reason(String),
    AccessDenied,
    General,
    Custom,
}

impl fmt::Display for ConnectorErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            ConnectorErrorKind::Encode(_) => write!(f, "encode error"),
            ConnectorErrorKind::Decode(_) => write!(f, "decode error"),
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
            ConnectorErrorKind::Encode(e) => Some(e),
            ConnectorErrorKind::Decode(e) => Some(e),
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
    fn encode(error: ironrdp_core::EncodeError) -> Self;
    fn decode(error: ironrdp_core::DecodeError) -> Self;
    fn general(context: &'static str) -> Self;
    fn reason(context: &'static str, reason: impl Into<String>) -> Self;
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
}

impl ConnectorErrorExt for ConnectorError {
    fn encode(error: ironrdp_core::EncodeError) -> Self {
        Self::new("encode error", ConnectorErrorKind::Encode(error))
    }

    fn decode(error: ironrdp_core::DecodeError) -> Self {
        Self::new("decode error", ConnectorErrorKind::Decode(error))
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

pub fn encode_x224_packet<T>(x224_msg: &T, buf: &mut WriteBuf) -> ConnectorResult<usize>
where
    T: Encode,
{
    let x224_msg_buf = encode_vec(x224_msg).map_err(ConnectorError::encode)?;

    let pdu = x224::X224Data {
        data: std::borrow::Cow::Owned(x224_msg_buf),
    };

    let written = encode_buf(&X224(pdu), buf).map_err(ConnectorError::encode)?;

    Ok(written)
}
