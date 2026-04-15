mod target_addr;
pub use target_addr::{ParseTargetAddrError, TargetAddr, TargetHost};

use ironrdp_propertyset::PropertySet;

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
/// See also: <https://learn.microsoft.com/en-us/windows/win32/termserv/rdp-file-settings>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayUsageMethod {
    /// 0: Do not use an RD Gateway server.
    Direct,
    /// 1: Always use an RD Gateway server.
    UseAlways,
    /// 2: Use an RD Gateway server, bypass for local addresses.
    UseBypassLocal,
    /// 3: Use an RD Gateway server, never bypass.
    UseNeverBypass,
    /// 4: Automatically detect RD Gateway settings (client-side heuristic; no explicit gateway configured).
    Automatic,
}

impl GatewayUsageMethod {
    /// Returns `true` when the file explicitly requires routing through a gateway server.
    pub fn is_gateway_required(self) -> bool {
        matches!(self, Self::UseAlways | Self::UseBypassLocal | Self::UseNeverBypass)
    }

    /// Returns the raw integer value for writing to a `.rdp` property set.
    pub fn as_i64(self) -> i64 {
        match self {
            Self::Direct => 0,
            Self::UseAlways => 1,
            Self::UseBypassLocal => 2,
            Self::UseNeverBypass => 3,
            Self::Automatic => 4,
        }
    }
}

impl TryFrom<i64> for GatewayUsageMethod {
    type Error = UnknownGatewayUsageMethod;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Direct),
            1 => Ok(Self::UseAlways),
            2 => Ok(Self::UseBypassLocal),
            3 => Ok(Self::UseNeverBypass),
            4 => Ok(Self::Automatic),
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
pub enum GatewayCredentialsSource {
    /// 0: Use the same credentials as the RDP server (pass-through / NTLM).
    UseServerCredentials,
    /// 1: Use the gateway-specific user credentials.
    UseUserCredentials,
    /// 2: Use credentials stored in a profile.
    UseProfile,
    /// 3: Prompt the user for gateway credentials.
    Prompt,
    /// 4: Use a smart card.
    SmartCard,
    /// 5: Use the logged-on user's credentials.
    UseLogonCredentials,
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
pub enum AudioMode {
    /// 0: Redirect audio to the local (client) machine.
    RedirectToClient,
    /// 1: Play audio on the remote computer.
    PlayOnServer,
    /// 2: Do not play audio.
    Disabled,
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

/// Error returned when an `audiomode` value is not a recognized variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownAudioMode(pub i64);

impl core::fmt::Display for UnknownAudioMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unknown audiomode value: {}", self.0)
    }
}

impl core::error::Error for UnknownAudioMode {}

pub trait PropertySetExt {
    fn full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError>;

    fn server_port(&self) -> Result<Option<u16>, InvalidServerPort>;

    fn alternate_full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError>;

    fn domain(&self) -> Option<&str>;

    fn enable_credssp_support(&self) -> Option<bool>;

    fn compression(&self) -> Option<bool>;

    fn gateway_hostname(&self) -> Option<&str>;

    fn gateway_usage_method(&self) -> Result<Option<GatewayUsageMethod>, UnknownGatewayUsageMethod>;

    fn gateway_credentials_source(&self) -> Result<Option<GatewayCredentialsSource>, UnknownGatewayCredentialsSource>;

    fn gateway_username(&self) -> Option<&str>;

    fn gateway_password(&self) -> Option<&str>;

    fn desktop_width(&self) -> Result<Option<u16>, InvalidDesktopSize>;

    fn desktop_height(&self) -> Result<Option<u16>, InvalidDesktopSize>;

    fn desktop_scale_factor(&self) -> Result<Option<u32>, InvalidDesktopSize>;

    fn alternate_shell(&self) -> Option<&str>;

    fn shell_working_directory(&self) -> Option<&str>;

    fn redirect_clipboard(&self) -> Option<bool>;

    fn audio_mode(&self) -> Result<Option<AudioMode>, UnknownAudioMode>;

    fn remote_application_name(&self) -> Option<&str>;

    fn remote_application_program(&self) -> Option<&str>;

    fn kdc_proxy_url(&self) -> Option<&str>;

    fn kdc_proxy_name(&self) -> Option<&str>;

    fn username(&self) -> Option<&str>;

    /// Target RDP server password - use for testing only
    fn clear_text_password(&self) -> Option<&str>;
}

impl PropertySetExt for PropertySet {
    fn full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError> {
        self.get::<&str>("full address").map(|s| s.parse()).transpose()
    }

    fn server_port(&self) -> Result<Option<u16>, InvalidServerPort> {
        self.get::<i64>("server port")
            .map(|p| u16::try_from(p).ok().filter(|&p| p != 0).ok_or(InvalidServerPort))
            .transpose()
    }

    fn alternate_full_address(&self) -> Result<Option<TargetAddr>, ParseTargetAddrError> {
        self.get::<&str>("alternate full address")
            .map(|s| s.parse())
            .transpose()
    }

    fn domain(&self) -> Option<&str> {
        self.get::<&str>("domain")
    }

    fn enable_credssp_support(&self) -> Option<bool> {
        self.get::<bool>("enablecredsspsupport")
    }

    fn compression(&self) -> Option<bool> {
        self.get::<bool>("compression")
    }

    fn gateway_hostname(&self) -> Option<&str> {
        self.get::<&str>("gatewayhostname")
    }

    fn gateway_usage_method(&self) -> Result<Option<GatewayUsageMethod>, UnknownGatewayUsageMethod> {
        self.get::<i64>("gatewayusagemethod")
            .map(GatewayUsageMethod::try_from)
            .transpose()
    }

    fn gateway_credentials_source(&self) -> Result<Option<GatewayCredentialsSource>, UnknownGatewayCredentialsSource> {
        self.get::<i64>("gatewaycredentialssource")
            .map(GatewayCredentialsSource::try_from)
            .transpose()
    }

    fn gateway_username(&self) -> Option<&str> {
        self.get::<&str>("gatewayusername")
    }

    fn gateway_password(&self) -> Option<&str> {
        self.get::<&str>("GatewayPassword")
            .or_else(|| self.get::<&str>("gatewaypassword"))
    }

    fn desktop_width(&self) -> Result<Option<u16>, InvalidDesktopSize> {
        self.get::<i64>("desktopwidth")
            .map(|v| u16::try_from(v).map_err(|_| InvalidDesktopSize))
            .transpose()
    }

    fn desktop_height(&self) -> Result<Option<u16>, InvalidDesktopSize> {
        self.get::<i64>("desktopheight")
            .map(|v| u16::try_from(v).map_err(|_| InvalidDesktopSize))
            .transpose()
    }

    fn desktop_scale_factor(&self) -> Result<Option<u32>, InvalidDesktopSize> {
        self.get::<i64>("desktopscalefactor")
            .map(|v| u32::try_from(v).map_err(|_| InvalidDesktopSize))
            .transpose()
    }

    fn alternate_shell(&self) -> Option<&str> {
        self.get::<&str>("alternate shell")
    }

    fn shell_working_directory(&self) -> Option<&str> {
        self.get::<&str>("shell working directory")
    }

    fn redirect_clipboard(&self) -> Option<bool> {
        self.get::<bool>("redirectclipboard")
    }

    fn audio_mode(&self) -> Result<Option<AudioMode>, UnknownAudioMode> {
        self.get::<i64>("audiomode").map(AudioMode::try_from).transpose()
    }

    fn remote_application_name(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationname")
    }

    fn remote_application_program(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationprogram")
    }

    fn kdc_proxy_url(&self) -> Option<&str> {
        self.get::<&str>("kdcproxyurl")
            .or_else(|| self.get::<&str>("KDCProxyURL"))
    }

    fn kdc_proxy_name(&self) -> Option<&str> {
        self.get::<&str>("kdcproxyname")
    }

    fn username(&self) -> Option<&str> {
        self.get::<&str>("username")
    }

    fn clear_text_password(&self) -> Option<&str> {
        self.get::<&str>("ClearTextPassword")
    }
}
