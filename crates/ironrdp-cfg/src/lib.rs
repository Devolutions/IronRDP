mod target_addr;
use std::path::PathBuf;

pub use target_addr::{ParseTargetAddrError, TargetAddr, TargetHost};

use ironrdp_propertyset::PropertySet;

/// Property keys whose values are secrets and must never be surfaced verbatim.
///
/// Matching is case-insensitive, so a single lowercase entry covers casing variants such as
/// `GatewayPassword`/`gatewaypassword` and `ClearTextPassword`/`cleartextpassword`.
const SECRET_KEYS: &[&str] = &[
    "cleartextpassword",        // plaintext RDP account password
    "gatewaypassword",          // RD gateway password (both casings)
    "ironrdp_rdcleanpathtoken", // RDCleanPath authentication token
];

/// Returns `true` when `key` names a property whose value is a secret (password or token).
///
/// Consumers that expose property sets to untrusted readers (logs, IPC responses, dumps) should
/// redact the value of any key for which this returns `true`. The comparison is case-insensitive.
pub fn is_secret_key(key: &str) -> bool {
    SECRET_KEYS.iter().any(|secret| key.eq_ignore_ascii_case(secret))
}

/// Error returned when the `server port` property value is outside the valid port range (1–65535).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidServerPort;

impl core::fmt::Display for InvalidServerPort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("server port value is out of the valid port range (1-65535)")
    }
}

impl core::error::Error for InvalidServerPort {}

/// Error returned when a desktop dimension or scale factor property value is out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidDesktopSize;

impl core::fmt::Display for InvalidDesktopSize {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("desktop size property value is out of range")
    }
}

impl core::error::Error for InvalidDesktopSize {}

/// Controls whether and how an RD Gateway server is used.
///
/// Corresponds to the `gatewayusagemethod` `.rdp` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i64)]
pub enum GatewayUsageMethod {
    /// Do not use an RD Gateway server.
    ///
    /// RDC UI: "Bypass RD Gateway server for local addresses" is cleared.
    Direct = 0,

    /// Always use the RD Gateway server.
    ///
    /// RDC UI: bypass-local is cleared.
    UseAlways = 1,

    /// Use an RD Gateway server if a direct connection cannot be made.
    ///
    /// Windows semantics are "try direct, use gateway if direct fails".
    ///
    /// IronRDP currently does not implement that two-step fallback, and if
    /// an explicit gateway hostname is present, it selects it eagerly as the best
    /// available approximation.
    ///
    /// RDC UI: bypass-local is selected.
    #[default]
    Detect = 2,

    /// Use the default RD Gateway settings.
    UseDefaultSettings = 3,

    /// Do not use an RD Gateway server.
    ///
    /// RDC UI: bypass-local is selected.
    DirectBypassLocal = 4,
}

impl GatewayUsageMethod {
    /// Returns `true` when the file explicitly requires routing through a gateway server.
    ///
    /// This is only true for `gatewayusagemethod:i:1`.
    /// `Detect` / value 2 may use a gateway, but does not require one.
    /// `UseDefaultSettings` / value 3 delegates the decision to client/default policy.
    pub fn is_gateway_required(self) -> bool {
        matches!(self, Self::UseAlways)
    }

    /// Returns `true` when this mode may result in gateway usage.
    ///
    /// This includes explicit gateway use, detect/on-demand gateway use,
    /// and default settings, because defaults or policy may require a gateway.
    pub fn may_use_gateway(self) -> bool {
        matches!(self, Self::UseAlways | Self::Detect | Self::UseDefaultSettings)
    }

    /// Returns the raw integer value for writing to a `.rdp` property set.
    #[expect(
        clippy::as_conversions,
        reason = "the enum is #[repr(i64)] with explicit discriminants"
    )]
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

impl TryFrom<i64> for GatewayUsageMethod {
    type Error = UnknownGatewayUsageMethod;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Direct),
            1 => Ok(Self::UseAlways),
            2 => Ok(Self::Detect),
            3 => Ok(Self::UseDefaultSettings),
            4 => Ok(Self::DirectBypassLocal),
            _ => Err(UnknownGatewayUsageMethod(value)),
        }
    }
}

/// Error returned when a `gatewayusagemethod` value is not a recognized variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownGatewayUsageMethod(pub i64);

impl core::fmt::Display for UnknownGatewayUsageMethod {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unknown gatewayusagemethod value: {}", self.0)
    }
}

impl core::error::Error for UnknownGatewayUsageMethod {}

/// Controls which credentials are used to authenticate to the RD Gateway.
///
/// Corresponds to the `gatewaycredentialssource` `.rdp` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum GatewayCredentialsSource {
    /// 0: Use the same credentials as the RDP server (pass-through / NTLM).
    UseServerCredentials = 0,
    /// 1: Use the gateway-specific user credentials.
    UseUserCredentials = 1,
    /// 2: Use credentials stored in a profile.
    UseProfile = 2,
    /// 3: Prompt the user for gateway credentials.
    Prompt = 3,
    /// 4: Use a smart card.
    SmartCard = 4,
    /// 5: Use the logged-on user's credentials.
    UseLogonCredentials = 5,
}

impl TryFrom<i64> for GatewayCredentialsSource {
    type Error = UnknownGatewayCredentialsSource;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::UseServerCredentials),
            1 => Ok(Self::UseUserCredentials),
            2 => Ok(Self::UseProfile),
            3 => Ok(Self::Prompt),
            4 => Ok(Self::SmartCard),
            5 => Ok(Self::UseLogonCredentials),
            _ => Err(UnknownGatewayCredentialsSource(value)),
        }
    }
}

impl GatewayCredentialsSource {
    /// Returns the raw integer value for writing to a `.rdp` property set.
    #[expect(
        clippy::as_conversions,
        reason = "the enum is #[repr(i64)] with explicit discriminants"
    )]
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

/// Error returned when a `gatewaycredentialssource` value is not a recognized variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownGatewayCredentialsSource(pub i64);

impl core::fmt::Display for UnknownGatewayCredentialsSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unknown gatewaycredentialssource value: {}", self.0)
    }
}

impl core::error::Error for UnknownGatewayCredentialsSource {}

/// Controls where audio is played during a remote session.
///
/// Corresponds to the `audiomode` `.rdp` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum AudioMode {
    /// 0: Redirect audio to the local (client) machine.
    RedirectToClient = 0,
    /// 1: Play audio on the remote computer.
    PlayOnServer = 1,
    /// 2: Do not play audio.
    Disabled = 2,
}

impl TryFrom<i64> for AudioMode {
    type Error = UnknownAudioMode;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::RedirectToClient),
            1 => Ok(Self::PlayOnServer),
            2 => Ok(Self::Disabled),
            _ => Err(UnknownAudioMode(value)),
        }
    }
}

impl AudioMode {
    /// Returns the raw integer value for writing to a `.rdp` property set.
    #[expect(
        clippy::as_conversions,
        reason = "the enum is #[repr(i64)] with explicit discriminants"
    )]
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

/// Error returned when an `audiomode` value is not a recognized variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownAudioMode(pub i64);

impl core::fmt::Display for UnknownAudioMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unknown audiomode value: {}", self.0)
    }
}

impl core::error::Error for UnknownAudioMode {}

/// Name-to-pipe mapping for a single DVC proxy channel.
#[derive(Clone, Debug)]
pub struct DvcPipeProxy {
    pub channel_name: String,
    pub pipe_name: String,
}

/// Error returned when a desktop dimension or scale factor property value is out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DvcPipeSpecMissingDelimiter;

impl core::fmt::Display for DvcPipeSpecMissingDelimiter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DVC pipe proxy spec is missing the '=' delimiter")
    }
}

impl core::error::Error for DvcPipeSpecMissingDelimiter {}

/// Typed accessors for the RDP properties IronRDP understands.
///
/// Every property is exposed as a triplet of methods sharing the same underlying key: a getter
/// returning the parsed value (if present and valid), a `set_*` mutator writing it, and a `clear_*`
/// mutator removing it.
///
/// Methods are grouped into three sections:
///
/// - **Microsoft standard keys** — keys defined by the `.rdp` file format and Microsoft tooling.
/// - **IronRDP extensions** — IronRDP-specific keys, prefixed with `ironrdp_` to avoid colliding
///   with Microsoft keys.
/// - **Multi-key helpers** — convenience mutators acting on several related keys at once.
///
/// Within each section, properties are ordered alphabetically by their getter name.
pub trait PropertySetExt {
    // ── Microsoft standard keys ───────────────────────────────────────────────

    /// Alternate target server address (`alternate full address`).
    fn alternate_full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError>;
    /// Sets the `alternate full address` property.
    fn set_alternate_full_address(&mut self, value: &TargetAddr);
    /// Removes the `alternate full address` property.
    fn clear_alternate_full_address(&mut self);

    /// Alternate shell to launch on the server instead of the desktop (`alternate shell`).
    fn alternate_shell(&self) -> Option<&str>;
    /// Sets the `alternate shell` property.
    fn set_alternate_shell(&mut self, value: impl Into<String>);
    /// Removes the `alternate shell` property.
    fn clear_alternate_shell(&mut self);

    /// Audio output redirection mode (`audiomode`).
    fn audio_mode(&self) -> Result<Option<AudioMode>, UnknownAudioMode>;
    /// Sets the `audiomode` property.
    fn set_audio_mode(&mut self, value: AudioMode);
    /// Removes the `audiomode` property.
    fn clear_audio_mode(&mut self);

    /// Target RDP server password in clear text (`ClearTextPassword`).
    ///
    /// This is an MsRdpEx addition and a secret; use for testing only.
    fn clear_text_password(&self) -> Option<&str>;
    /// Sets the `ClearTextPassword` property.
    fn set_clear_text_password(&mut self, value: impl Into<String>);
    /// Removes the `ClearTextPassword` property.
    fn clear_clear_text_password(&mut self);

    /// Whether bulk compression is enabled (`compression`).
    fn compression(&self) -> Option<bool>;
    /// Sets the `compression` property.
    fn set_compression(&mut self, value: bool);
    /// Removes the `compression` property.
    fn clear_compression(&mut self);

    /// Requested desktop height in pixels (`desktopheight`).
    fn desktop_height(&self) -> Result<Option<u16>, InvalidDesktopSize>;
    /// Sets the `desktopheight` property.
    fn set_desktop_height(&mut self, value: u16);
    /// Removes the `desktopheight` property.
    fn clear_desktop_height(&mut self);

    /// Requested desktop scale factor as a percentage (`desktopscalefactor`).
    fn desktop_scale_factor(&self) -> Result<Option<u32>, InvalidDesktopSize>;
    /// Sets the `desktopscalefactor` property.
    fn set_desktop_scale_factor(&mut self, value: u32);
    /// Removes the `desktopscalefactor` property.
    fn clear_desktop_scale_factor(&mut self);

    /// Requested desktop width in pixels (`desktopwidth`).
    fn desktop_width(&self) -> Result<Option<u16>, InvalidDesktopSize>;
    /// Sets the `desktopwidth` property.
    fn set_desktop_width(&mut self, value: u16);
    /// Removes the `desktopwidth` property.
    fn clear_desktop_width(&mut self);

    /// Domain of the RDP account credentials (`domain`).
    fn domain(&self) -> Option<&str>;
    /// Sets the `domain` property.
    fn set_domain(&mut self, value: String);
    /// Removes the `domain` property.
    fn clear_domain(&mut self);

    /// Whether CredSSP/NLA support is enabled (`enablecredsspsupport`).
    fn enable_credssp_support(&self) -> Option<bool>;
    /// Sets the `enablecredsspsupport` property.
    fn set_enable_credssp_support(&mut self, enabled: bool);
    /// Removes the `enablecredsspsupport` property.
    fn clear_enable_credssp_support(&mut self);

    /// Target server address (`full address`).
    fn full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError>;
    /// Sets the `full address` property.
    fn set_full_address(&mut self, value: &TargetAddr);
    /// Removes the `full address` property.
    fn clear_full_address(&mut self);

    /// RD gateway credentials source (`gatewaycredentialssource`).
    fn gateway_credentials_source(&self) -> Result<Option<GatewayCredentialsSource>, UnknownGatewayCredentialsSource>;
    /// Sets the `gatewaycredentialssource` property.
    fn set_gateway_credentials_source(&mut self, value: GatewayCredentialsSource);
    /// Removes the `gatewaycredentialssource` property.
    fn clear_gateway_credentials_source(&mut self);

    /// RD gateway endpoint hostname (`gatewayhostname`).
    fn gateway_hostname(&self) -> Option<&str>;
    /// Sets the `gatewayhostname` property.
    fn set_gateway_hostname(&mut self, value: impl Into<String>);
    /// Removes the `gatewayhostname` property.
    fn clear_gateway_hostname(&mut self);

    /// RD gateway password (`GatewayPassword`; secret).
    ///
    /// Reads either the `GatewayPassword` or `gatewaypassword` casing.
    fn gateway_password(&self) -> Option<&str>;
    /// Sets the `GatewayPassword` property (and removes the `gatewaypassword` casing).
    fn set_gateway_password(&mut self, value: impl Into<String>);
    /// Removes the `GatewayPassword` property (both casings).
    fn clear_gateway_password(&mut self);

    /// RD gateway usage method (`gatewayusagemethod`).
    fn gateway_usage_method(&self) -> Result<Option<GatewayUsageMethod>, UnknownGatewayUsageMethod>;
    /// Sets the `gatewayusagemethod` property.
    fn set_gateway_usage_method(&mut self, value: GatewayUsageMethod);
    /// Removes the `gatewayusagemethod` property.
    fn clear_gateway_usage_method(&mut self);

    /// RD gateway username (`gatewayusername`).
    fn gateway_username(&self) -> Option<&str>;
    /// Sets the `gatewayusername` property.
    fn set_gateway_username(&mut self, value: impl Into<String>);
    /// Removes the `gatewayusername` property.
    fn clear_gateway_username(&mut self);

    /// Kerberos KDC proxy name (`kdcproxyname`).
    fn kdc_proxy_name(&self) -> Option<&str>;
    /// Sets the `kdcproxyname` property.
    fn set_kdc_proxy_name(&mut self, value: impl Into<String>);
    /// Removes the `kdcproxyname` property.
    fn clear_kdc_proxy_name(&mut self);

    /// Kerberos KDC proxy URL (`kdcproxyurl`).
    ///
    /// Reads either the `kdcproxyurl` or `KDCProxyURL` casing.
    fn kdc_proxy_url(&self) -> Option<&str>;
    /// Sets the `kdcproxyurl` property (and removes the `KDCProxyURL` casing).
    fn set_kdc_proxy_url(&mut self, value: impl Into<String>);
    /// Removes the `kdcproxyurl` property (both casings).
    fn clear_kdc_proxy_url(&mut self);

    /// Whether clipboard redirection is requested (`redirectclipboard`).
    fn redirect_clipboard(&self) -> Option<bool>;
    /// Sets the `redirectclipboard` property.
    fn set_redirect_clipboard(&mut self, value: bool);
    /// Removes the `redirectclipboard` property.
    fn clear_redirect_clipboard(&mut self);

    /// RemoteApp application name (`remoteapplicationname`).
    fn remote_application_name(&self) -> Option<&str>;
    /// Sets the `remoteapplicationname` property.
    fn set_remote_application_name(&mut self, value: impl Into<String>);
    /// Removes the `remoteapplicationname` property.
    fn clear_remote_application_name(&mut self);

    /// RemoteApp executable path or alias (`remoteapplicationprogram`).
    fn remote_application_program(&self) -> Option<&str>;
    /// Sets the `remoteapplicationprogram` property.
    fn set_remote_application_program(&mut self, value: impl Into<String>);
    /// Removes the `remoteapplicationprogram` property.
    fn clear_remote_application_program(&mut self);

    /// Target server port (`server port`).
    fn server_port(&self) -> Result<Option<u16>, InvalidServerPort>;
    /// Sets the `server port` property.
    fn set_server_port(&mut self, value: u16);
    /// Removes the `server port` property.
    fn clear_server_port(&mut self);

    /// Working directory for the alternate shell (`shell working directory`).
    fn shell_working_directory(&self) -> Option<&str>;
    /// Sets the `shell working directory` property.
    fn set_shell_working_directory(&mut self, value: impl Into<String>);
    /// Removes the `shell working directory` property.
    fn clear_shell_working_directory(&mut self);

    /// Username of the RDP account credentials (`username`).
    fn username(&self) -> Option<&str>;
    /// Sets the `username` property.
    fn set_username(&mut self, value: impl Into<String>);
    /// Removes the `username` property.
    fn clear_username(&mut self);

    // ── IronRDP extensions ────────────────────────────────────────────────────

    /// Automatically log on by passing the `INFO_AUTOLOGON` flag (`ironrdp_autologon`).
    fn autologon(&self) -> Option<bool>;
    /// Sets the `ironrdp_autologon` property.
    fn set_autologon(&mut self, enabled: bool);
    /// Removes the `ironrdp_autologon` property.
    fn clear_autologon(&mut self);

    /// Color depth in bits per pixel, e.g. 16 or 32 (`ironrdp_colordepth`).
    fn color_depth(&self) -> Option<u32>;
    /// Sets the `ironrdp_colordepth` property.
    fn set_color_depth(&mut self, depth: u32);
    /// Removes the `ironrdp_colordepth` property.
    fn clear_color_depth(&mut self);

    /// Bulk compression level: 0=K8, 1=K64, 2=Rdp6, 3=Rdp61 (`ironrdp_compressionlevel`).
    fn compression_level(&self) -> Option<u32>;
    /// Sets the `ironrdp_compressionlevel` property.
    fn set_compression_level(&mut self, level: u32);
    /// Removes the `ironrdp_compressionlevel` property.
    fn clear_compression_level(&mut self);

    /// DVC pipe proxy specifications (`ironrdp_dvcpipeproxy`).
    ///
    /// The underlying value is a comma-separated list of `<name>=<pipe>` entries.
    fn dvc_pipe_proxies(&self) -> impl Iterator<Item = Result<DvcPipeProxy, DvcPipeSpecMissingDelimiter>>;
    /// Sets the `ironrdp_dvcpipeproxy` property from an iterator of specifications.
    fn set_dvc_pipe_proxies<T>(&mut self, specs: T)
    where
        T: IntoIterator<Item = DvcPipeProxy>;
    /// Removes the `ironrdp_dvcpipeproxy` property.
    fn clear_dvc_pipe_proxies(&mut self);

    /// DVC client plugin DLL paths, comma-separated; Windows only (`ironrdp_dvcplugin`).
    fn dvc_plugins(&self) -> impl Iterator<Item = PathBuf>;
    /// Sets the `ironrdp_dvcplugin` property from an iterator of paths.
    fn set_dvc_plugins<'a, T>(&mut self, paths: T)
    where
        T: IntoIterator<Item = &'a std::path::Path>;
    /// Removes the `ironrdp_dvcplugin` property.
    fn clear_dvc_plugins(&mut self);

    /// Enable the QOI bitmap codec (`ironrdp_qoi`).
    fn enable_qoi(&self) -> Option<bool>;
    /// Sets the `ironrdp_qoi` property.
    fn set_enable_qoi(&mut self, enabled: bool);
    /// Removes the `ironrdp_qoi` property.
    fn clear_enable_qoi(&mut self);

    /// Enable the QOIZ bitmap codec (`ironrdp_qoiz`).
    fn enable_qoiz(&self) -> Option<bool>;
    /// Sets the `ironrdp_qoiz` property.
    fn set_enable_qoiz(&mut self, enabled: bool);
    /// Removes the `ironrdp_qoiz` property.
    fn clear_enable_qoiz(&mut self);

    /// Enable RDPDR device redirection (`ironrdp_rdpdr`).
    fn enable_rdpdr(&self) -> Option<bool>;
    /// Sets the `ironrdp_rdpdr` property.
    fn set_enable_rdpdr(&mut self, enabled: bool);
    /// Removes the `ironrdp_rdpdr` property.
    fn clear_enable_rdpdr(&mut self);

    /// Enable smart-card redirection within RDPDR (`ironrdp_smartcard`).
    fn enable_smartcard(&self) -> Option<bool>;
    /// Sets the `ironrdp_smartcard` property.
    fn set_enable_smartcard(&mut self, enabled: bool);
    /// Removes the `ironrdp_smartcard` property.
    fn clear_enable_smartcard(&mut self);

    /// Enable TLS + graphical login; default enabled (`ironrdp_tls`).
    fn enable_tls(&self) -> Option<bool>;
    /// Sets the `ironrdp_tls` property.
    fn set_enable_tls(&mut self, enabled: bool);
    /// Removes the `ironrdp_tls` property.
    fn clear_enable_tls(&mut self);

    /// Idle anti-lock fake events interval in minutes (`ironrdp_fakeeventsinterval`).
    fn fake_events_interval(&self) -> Option<u32>;
    /// Sets the `ironrdp_fakeeventsinterval` property.
    fn set_fake_events_interval(&mut self, minutes: u32);
    /// Removes the `ironrdp_fakeeventsinterval` property.
    fn clear_fake_events_interval(&mut self);

    /// RDCleanPath authentication token; secret (`ironrdp_rdcleanpathtoken`).
    fn rdcleanpath_token(&self) -> Option<&str>;
    /// Sets the `ironrdp_rdcleanpathtoken` property.
    fn set_rdcleanpath_token(&mut self, value: impl Into<String>);
    /// Removes the `ironrdp_rdcleanpathtoken` property.
    fn clear_rdcleanpath_token(&mut self);

    /// RDCleanPath proxy URL (`ironrdp_rdcleanpathurl`).
    fn rdcleanpath_url(&self) -> Option<&str>;
    /// Sets the `ironrdp_rdcleanpathurl` property.
    fn set_rdcleanpath_url(&mut self, value: impl Into<String>);
    /// Removes the `ironrdp_rdcleanpathurl` property.
    fn clear_rdcleanpath_url(&mut self);

    /// Render the server-side pointer; default enabled (`ironrdp_serverpointer`).
    fn server_pointer(&self) -> Option<bool>;
    /// Sets the `ironrdp_serverpointer` property.
    fn set_server_pointer(&mut self, enabled: bool);
    /// Removes the `ironrdp_serverpointer` property.
    fn clear_server_pointer(&mut self);

    // ── Multi-key helpers ─────────────────────────────────────────────────────

    /// Removes every gateway-related key (`gatewayhostname`, `gatewayusagemethod`,
    /// `gatewayusername`, and both `GatewayPassword` casings).
    fn clear_gateway(&mut self);

    /// Removes every RDCleanPath-related key (`ironrdp_rdcleanpathurl` and
    /// `ironrdp_rdcleanpathtoken`).
    fn clear_rdcleanpath(&mut self);
}

impl PropertySetExt for PropertySet {
    // ── Microsoft standard keys ───────────────────────────────────────────────

    fn alternate_full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError> {
        self.get::<&str>("alternate full address")
            .map(|s| s.parse())
            .transpose()
    }

    fn set_alternate_full_address(&mut self, value: &TargetAddr) {
        self.insert("alternate full address", value.to_string());
    }

    fn clear_alternate_full_address(&mut self) {
        self.remove("alternate full address");
    }

    fn alternate_shell(&self) -> Option<&str> {
        self.get::<&str>("alternate shell")
    }

    fn set_alternate_shell(&mut self, value: impl Into<String>) {
        self.insert("alternate shell", value.into());
    }

    fn clear_alternate_shell(&mut self) {
        self.remove("alternate shell");
    }

    fn audio_mode(&self) -> Result<Option<AudioMode>, UnknownAudioMode> {
        self.get::<i64>("audiomode").map(AudioMode::try_from).transpose()
    }

    fn set_audio_mode(&mut self, value: AudioMode) {
        self.insert("audiomode", value.as_i64());
    }

    fn clear_audio_mode(&mut self) {
        self.remove("audiomode");
    }

    fn clear_text_password(&self) -> Option<&str> {
        self.get::<&str>("ClearTextPassword")
    }

    fn set_clear_text_password(&mut self, value: impl Into<String>) {
        self.insert("ClearTextPassword", value.into());
    }

    fn clear_clear_text_password(&mut self) {
        self.remove("ClearTextPassword");
    }

    fn compression(&self) -> Option<bool> {
        self.get::<bool>("compression")
    }

    fn set_compression(&mut self, value: bool) {
        self.insert("compression", value);
    }

    fn clear_compression(&mut self) {
        self.remove("compression");
    }

    fn desktop_height(&self) -> Result<Option<u16>, InvalidDesktopSize> {
        self.get::<i64>("desktopheight")
            .map(|v| u16::try_from(v).map_err(|_| InvalidDesktopSize))
            .transpose()
    }

    fn set_desktop_height(&mut self, value: u16) {
        self.insert("desktopheight", value);
    }

    fn clear_desktop_height(&mut self) {
        self.remove("desktopheight");
    }

    fn desktop_scale_factor(&self) -> Result<Option<u32>, InvalidDesktopSize> {
        self.get::<i64>("desktopscalefactor")
            .map(|v| u32::try_from(v).map_err(|_| InvalidDesktopSize))
            .transpose()
    }

    fn set_desktop_scale_factor(&mut self, value: u32) {
        self.insert("desktopscalefactor", value);
    }

    fn clear_desktop_scale_factor(&mut self) {
        self.remove("desktopscalefactor");
    }

    fn desktop_width(&self) -> Result<Option<u16>, InvalidDesktopSize> {
        self.get::<i64>("desktopwidth")
            .map(|v| u16::try_from(v).map_err(|_| InvalidDesktopSize))
            .transpose()
    }

    fn set_desktop_width(&mut self, value: u16) {
        self.insert("desktopwidth", value);
    }

    fn clear_desktop_width(&mut self) {
        self.remove("desktopwidth");
    }

    fn domain(&self) -> Option<&str> {
        self.get::<&str>("domain")
    }

    fn set_domain(&mut self, value: String) {
        self.insert("domain", value);
    }

    fn clear_domain(&mut self) {
        self.remove("domain");
    }

    fn enable_credssp_support(&self) -> Option<bool> {
        self.get::<bool>("enablecredsspsupport")
    }

    fn set_enable_credssp_support(&mut self, enabled: bool) {
        self.insert("enablecredsspsupport", i64::from(enabled));
    }

    fn clear_enable_credssp_support(&mut self) {
        self.remove("enablecredsspsupport");
    }

    fn full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError> {
        self.get::<&str>("full address").map(|s| s.parse()).transpose()
    }

    fn set_full_address(&mut self, value: &TargetAddr) {
        self.insert("full address", value.to_string());
    }

    fn clear_full_address(&mut self) {
        self.remove("full address");
    }

    fn gateway_credentials_source(&self) -> Result<Option<GatewayCredentialsSource>, UnknownGatewayCredentialsSource> {
        self.get::<i64>("gatewaycredentialssource")
            .map(GatewayCredentialsSource::try_from)
            .transpose()
    }

    fn set_gateway_credentials_source(&mut self, value: GatewayCredentialsSource) {
        self.insert("gatewaycredentialssource", value.as_i64());
    }

    fn clear_gateway_credentials_source(&mut self) {
        self.remove("gatewaycredentialssource");
    }

    fn gateway_hostname(&self) -> Option<&str> {
        self.get::<&str>("gatewayhostname")
    }

    fn set_gateway_hostname(&mut self, value: impl Into<String>) {
        self.insert("gatewayhostname", value.into());
    }

    fn clear_gateway_hostname(&mut self) {
        self.remove("gatewayhostname");
    }

    fn gateway_password(&self) -> Option<&str> {
        self.get::<&str>("GatewayPassword")
            .or_else(|| self.get::<&str>("gatewaypassword"))
    }

    fn set_gateway_password(&mut self, value: impl Into<String>) {
        self.insert("GatewayPassword", value.into());
        self.remove("gatewaypassword");
    }

    fn clear_gateway_password(&mut self) {
        self.remove("GatewayPassword");
        self.remove("gatewaypassword");
    }

    fn gateway_usage_method(&self) -> Result<Option<GatewayUsageMethod>, UnknownGatewayUsageMethod> {
        self.get::<i64>("gatewayusagemethod")
            .map(GatewayUsageMethod::try_from)
            .transpose()
    }

    fn set_gateway_usage_method(&mut self, value: GatewayUsageMethod) {
        self.insert("gatewayusagemethod", value.as_i64());
    }

    fn clear_gateway_usage_method(&mut self) {
        self.remove("gatewayusagemethod");
    }

    fn gateway_username(&self) -> Option<&str> {
        self.get::<&str>("gatewayusername")
    }

    fn set_gateway_username(&mut self, value: impl Into<String>) {
        self.insert("gatewayusername", value.into());
    }

    fn clear_gateway_username(&mut self) {
        self.remove("gatewayusername");
    }

    fn kdc_proxy_name(&self) -> Option<&str> {
        self.get::<&str>("kdcproxyname")
    }

    fn set_kdc_proxy_name(&mut self, value: impl Into<String>) {
        self.insert("kdcproxyname", value.into());
    }

    fn clear_kdc_proxy_name(&mut self) {
        self.remove("kdcproxyname");
    }

    fn kdc_proxy_url(&self) -> Option<&str> {
        self.get::<&str>("kdcproxyurl")
            .or_else(|| self.get::<&str>("KDCProxyURL"))
    }

    fn set_kdc_proxy_url(&mut self, value: impl Into<String>) {
        self.insert("kdcproxyurl", value.into());
        self.remove("KDCProxyURL");
    }

    fn clear_kdc_proxy_url(&mut self) {
        self.remove("kdcproxyurl");
        self.remove("KDCProxyURL");
    }

    fn redirect_clipboard(&self) -> Option<bool> {
        self.get::<bool>("redirectclipboard")
    }

    fn set_redirect_clipboard(&mut self, value: bool) {
        self.insert("redirectclipboard", value);
    }

    fn clear_redirect_clipboard(&mut self) {
        self.remove("redirectclipboard");
    }

    fn remote_application_name(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationname")
    }

    fn set_remote_application_name(&mut self, value: impl Into<String>) {
        self.insert("remoteapplicationname", value.into());
    }

    fn clear_remote_application_name(&mut self) {
        self.remove("remoteapplicationname");
    }

    fn remote_application_program(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationprogram")
    }

    fn set_remote_application_program(&mut self, value: impl Into<String>) {
        self.insert("remoteapplicationprogram", value.into());
    }

    fn clear_remote_application_program(&mut self) {
        self.remove("remoteapplicationprogram");
    }

    fn server_port(&self) -> Result<Option<u16>, InvalidServerPort> {
        self.get::<i64>("server port")
            .map(|p| u16::try_from(p).ok().filter(|&p| p != 0).ok_or(InvalidServerPort))
            .transpose()
    }

    fn set_server_port(&mut self, value: u16) {
        self.insert("server port", value);
    }

    fn clear_server_port(&mut self) {
        self.remove("server port");
    }

    fn shell_working_directory(&self) -> Option<&str> {
        self.get::<&str>("shell working directory")
    }

    fn set_shell_working_directory(&mut self, value: impl Into<String>) {
        self.insert("shell working directory", value.into());
    }

    fn clear_shell_working_directory(&mut self) {
        self.remove("shell working directory");
    }

    fn username(&self) -> Option<&str> {
        self.get::<&str>("username")
    }

    fn set_username(&mut self, value: impl Into<String>) {
        self.insert("username", value.into());
    }

    fn clear_username(&mut self) {
        self.remove("username");
    }

    // ── IronRDP extensions ────────────────────────────────────────────────────

    fn autologon(&self) -> Option<bool> {
        self.get::<bool>("ironrdp_autologon")
    }

    fn set_autologon(&mut self, enabled: bool) {
        self.insert("ironrdp_autologon", enabled);
    }

    fn clear_autologon(&mut self) {
        self.remove("ironrdp_autologon");
    }

    fn color_depth(&self) -> Option<u32> {
        self.get::<u32>("ironrdp_colordepth")
    }

    fn set_color_depth(&mut self, depth: u32) {
        self.insert("ironrdp_colordepth", i64::from(depth));
    }

    fn clear_color_depth(&mut self) {
        self.remove("ironrdp_colordepth");
    }

    fn compression_level(&self) -> Option<u32> {
        self.get::<u32>("ironrdp_compressionlevel")
    }

    fn set_compression_level(&mut self, level: u32) {
        self.insert("ironrdp_compressionlevel", i64::from(level));
    }

    fn clear_compression_level(&mut self) {
        self.remove("ironrdp_compressionlevel");
    }

    fn dvc_pipe_proxies(&self) -> impl Iterator<Item = Result<DvcPipeProxy, DvcPipeSpecMissingDelimiter>> {
        self.get::<&str>("ironrdp_dvcpipeproxy")
            .into_iter()
            .flat_map(|value| value.split(','))
            .filter_map(|mut mapping| {
                mapping = mapping.trim();

                if mapping.is_empty() {
                    return None;
                }

                match mapping.split_once('=') {
                    Some((channel, pipe)) => Some(Ok(DvcPipeProxy {
                        channel_name: channel.to_owned(),
                        pipe_name: pipe.to_owned(),
                    })),
                    None => Some(Err(DvcPipeSpecMissingDelimiter)),
                }
            })
    }

    fn set_dvc_pipe_proxies<T>(&mut self, specs: T)
    where
        T: IntoIterator<Item = DvcPipeProxy>,
    {
        let mut value = String::new();

        for spec in specs {
            if !value.is_empty() {
                value.push(',');
            }

            value.push_str(&spec.channel_name);
            value.push('=');
            value.push_str(&spec.pipe_name);
        }

        self.insert("ironrdp_dvcpipeproxy", value);
    }

    fn clear_dvc_pipe_proxies(&mut self) {
        self.remove("ironrdp_dvcpipeproxy");
    }

    fn dvc_plugins(&self) -> impl Iterator<Item = PathBuf> {
        self.get::<&str>("ironrdp_dvcplugin")
            .into_iter()
            .flat_map(|value| value.split(','))
            .map(PathBuf::from)
    }

    fn set_dvc_plugins<'a, T>(&mut self, paths: T)
    where
        T: IntoIterator<Item = &'a std::path::Path>,
    {
        let mut value = String::new();

        for path in paths.into_iter().flat_map(|path| path.to_str()) {
            if !value.is_empty() {
                value.push(',');
            }

            value.push_str(path);
        }

        self.insert("ironrdp_dvcplugin", value);
    }

    fn clear_dvc_plugins(&mut self) {
        self.remove("ironrdp_dvcplugin");
    }

    fn enable_qoi(&self) -> Option<bool> {
        self.get::<bool>("ironrdp_qoi")
    }

    fn set_enable_qoi(&mut self, enabled: bool) {
        self.insert("ironrdp_qoi", enabled);
    }

    fn clear_enable_qoi(&mut self) {
        self.remove("ironrdp_qoi");
    }

    fn enable_qoiz(&self) -> Option<bool> {
        self.get::<bool>("ironrdp_qoiz")
    }

    fn set_enable_qoiz(&mut self, enabled: bool) {
        self.insert("ironrdp_qoiz", enabled);
    }

    fn clear_enable_qoiz(&mut self) {
        self.remove("ironrdp_qoiz");
    }

    fn enable_rdpdr(&self) -> Option<bool> {
        self.get::<bool>("ironrdp_rdpdr")
    }

    fn set_enable_rdpdr(&mut self, enabled: bool) {
        self.insert("ironrdp_rdpdr", enabled);
    }

    fn clear_enable_rdpdr(&mut self) {
        self.remove("ironrdp_rdpdr");
    }

    fn enable_smartcard(&self) -> Option<bool> {
        self.get::<bool>("ironrdp_smartcard")
    }

    fn set_enable_smartcard(&mut self, enabled: bool) {
        self.insert("ironrdp_smartcard", enabled);
    }

    fn clear_enable_smartcard(&mut self) {
        self.remove("ironrdp_smartcard");
    }

    fn enable_tls(&self) -> Option<bool> {
        self.get::<bool>("ironrdp_tls")
    }

    fn set_enable_tls(&mut self, enabled: bool) {
        self.insert("ironrdp_tls", enabled);
    }

    fn clear_enable_tls(&mut self) {
        self.remove("ironrdp_tls");
    }

    fn fake_events_interval(&self) -> Option<u32> {
        self.get::<u32>("ironrdp_fakeeventsinterval")
    }

    fn set_fake_events_interval(&mut self, minutes: u32) {
        self.insert("ironrdp_fakeeventsinterval", i64::from(minutes));
    }

    fn clear_fake_events_interval(&mut self) {
        self.remove("ironrdp_fakeeventsinterval");
    }

    fn rdcleanpath_token(&self) -> Option<&str> {
        self.get::<&str>("ironrdp_rdcleanpathtoken")
    }

    fn set_rdcleanpath_token(&mut self, value: impl Into<String>) {
        self.insert("ironrdp_rdcleanpathtoken", value.into());
    }

    fn clear_rdcleanpath_token(&mut self) {
        self.remove("ironrdp_rdcleanpathtoken");
    }

    fn rdcleanpath_url(&self) -> Option<&str> {
        self.get::<&str>("ironrdp_rdcleanpathurl")
    }

    fn set_rdcleanpath_url(&mut self, value: impl Into<String>) {
        self.insert("ironrdp_rdcleanpathurl", value.into());
    }

    fn clear_rdcleanpath_url(&mut self) {
        self.remove("ironrdp_rdcleanpathurl");
    }

    fn server_pointer(&self) -> Option<bool> {
        self.get::<bool>("ironrdp_serverpointer")
    }

    fn set_server_pointer(&mut self, enabled: bool) {
        self.insert("ironrdp_serverpointer", enabled);
    }

    fn clear_server_pointer(&mut self) {
        self.remove("ironrdp_serverpointer");
    }

    // ── Multi-key helpers ─────────────────────────────────────────────────────

    fn clear_gateway(&mut self) {
        self.remove("gatewayhostname");
        self.remove("gatewayusagemethod");
        self.remove("gatewayusername");
        self.remove("gatewaypassword");
        self.remove("GatewayPassword");
    }

    fn clear_rdcleanpath(&mut self) {
        self.remove("ironrdp_rdcleanpathurl");
        self.remove("ironrdp_rdcleanpathtoken");
    }
}
