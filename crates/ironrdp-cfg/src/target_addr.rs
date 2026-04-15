use core::fmt;
use core::net::{IpAddr, Ipv6Addr};
use core::str::FromStr;

/// The host component of an RDP target address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetHost {
    /// A resolved IP address (IPv4 or IPv6).
    Ip(IpAddr),
    /// A hostname or domain name.
    Domain(String),
}

impl fmt::Display for TargetHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // IPv6 addresses must be bracketed in .rdp file format and in URI contexts.
            Self::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]"),
            Self::Ip(ip) => write!(f, "{ip}"),
            Self::Domain(host) => write!(f, "{host}"),
        }
    }
}

/// A parsed target address from an RDP file `full address` or `alternate full address` property.
///
/// The `.rdp` file format represents IPv6 addresses with square brackets — `[::1]` or
/// `[::1]:port`. This type handles all address variants (hostname, IPv4, bracketed IPv6)
/// with an optional embedded port.
///
/// When the port is absent ([`port`] is `None`), the `server port` property should be
/// consulted separately via [`PropertySetExt::server_port`].
///
/// [`PropertySetExt::server_port`]: crate::PropertySetExt::server_port
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetAddr {
    /// The host component.
    pub host: TargetHost,
    /// Port embedded in the address string, if any.
    ///
    /// This does not account for the `server port` property; callers must combine both.
    pub port: Option<u16>,
}

impl fmt::Display for TargetAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

/// Error returned when a `full address` string cannot be parsed as a [`TargetAddr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseTargetAddrError {
    /// A `[` was found with no matching `]`.
    UnclosedBracket,
    /// The content between `[` and `]` is not a valid IPv6 address.
    InvalidIpv6Addr,
    /// The port suffix is not a valid `u16`.
    InvalidPort,
    /// Unexpected characters follow the closing `]`.
    UnexpectedTrailing,
}

impl fmt::Display for ParseTargetAddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedBracket => f.write_str("unclosed '[' in RDP address"),
            Self::InvalidIpv6Addr => f.write_str("invalid IPv6 address in RDP address"),
            Self::InvalidPort => f.write_str("invalid port in RDP address"),
            Self::UnexpectedTrailing => f.write_str("unexpected characters after ']' in RDP address"),
        }
    }
}

impl core::error::Error for ParseTargetAddrError {}

impl FromStr for TargetAddr {
    type Err = ParseTargetAddrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Bracketed IPv6: "[addr]:port" or "[addr]"
        if let Some(rest) = s.strip_prefix('[') {
            let (ipv6_str, rest) = rest.split_once(']').ok_or(ParseTargetAddrError::UnclosedBracket)?;
            let ip: Ipv6Addr = ipv6_str.parse().map_err(|_| ParseTargetAddrError::InvalidIpv6Addr)?;
            let port = match rest {
                "" => None,
                s if s.starts_with(':') => {
                    let port_str = s.strip_prefix(':').expect("already checked starts_with ':'");
                    Some(port_str.parse::<u16>().map_err(|_| ParseTargetAddrError::InvalidPort)?)
                }
                _ => return Err(ParseTargetAddrError::UnexpectedTrailing),
            };
            return Ok(TargetAddr {
                host: TargetHost::Ip(IpAddr::V6(ip)),
                port,
            });
        }

        // Bare IP address (no port) — must be checked before rsplit_once(':') because unbracketed
        // IPv6 like "::1" or "fe80::1" would otherwise be misparsed (trailing segment treated as port).
        if let Ok(ip) = s.parse::<IpAddr>() {
            return Ok(TargetAddr {
                host: TargetHost::Ip(ip),
                port: None,
            });
        }

        // "hostname:port" — use rsplit_once to separate on the last colon.
        // Any colon present after a non-IP address is unambiguously a port separator in the
        // .rdp format, so a non-numeric or out-of-range suffix is an error rather than a
        // fallback to a bare hostname.
        if let Some((host, port_str)) = s.rsplit_once(':') {
            let port = port_str.parse::<u16>().map_err(|_| ParseTargetAddrError::InvalidPort)?;
            let host = if let Ok(ip) = host.parse::<IpAddr>() {
                TargetHost::Ip(ip)
            } else {
                TargetHost::Domain(host.to_owned())
            };
            return Ok(TargetAddr { host, port: Some(port) });
        }

        // Bare hostname without port.
        Ok(TargetAddr {
            host: TargetHost::Domain(s.to_owned()),
            port: None,
        })
    }
}
