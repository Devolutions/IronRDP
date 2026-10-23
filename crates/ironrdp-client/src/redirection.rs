//! Following a Server Redirection PDU ([MS-RDPBCGR] 3.2.5.13.1) to the target it describes.
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/

use core::fmt;

use ironrdp_connector::{ConnectorResult, Credentials, reason_err};
use ironrdp_pdu::rdp::server_redirection::{ServerRedirectionFlags, ServerRedirectionPdu};
use tracing::{debug, info, warn};

use crate::config::{Config, Destination, Transport};

/// Most redirections followed by one client run before giving up, which breaks redirection loops.
pub(crate) const MAX_SERVER_REDIRECTIONS: u32 = 8;

/// What a Server Redirection PDU changes for the next connection attempts.
#[derive(Clone, Default)]
pub(crate) struct RedirectionHop {
    /// Hosts to try for the TCP connection of a direct transport, in order of preference.
    ///
    /// Empty means the destination name itself.
    pub(crate) connect_hosts: Vec<String>,
    /// Session to reconnect to, sent as `RedirectedSessionID` in the Client Cluster Data.
    pub(crate) session_id: u32,
    /// Password cookie sent in the Client Info PDU.
    pub(crate) password_cookie: Option<Vec<u8>>,
    /// DER-encoded certificate of the target, as the redirecting server reported it.
    pub(crate) target_certificate: Option<Vec<u8>>,
}

impl fmt::Debug for RedirectionHop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedirectionHop")
            .field("connect_hosts", &self.connect_hosts)
            .field("session_id", &self.session_id)
            .field("password_cookie", &self.password_cookie.as_ref().map(Vec::len))
            .field("target_certificate", &self.target_certificate.as_ref().map(Vec::len))
            .finish()
    }
}

/// Updates `config` so that the next connection goes to the target of `redirection`.
///
/// Returns the name of the target, for display.
///
/// The transport, TLS and CredSSP settings are kept: the target is authenticated with the same credentials, with the
/// user name and domain replaced when the PDU carries them.
pub(crate) fn apply_server_redirection(
    config: &mut Config,
    redirection: &ServerRedirectionPdu,
) -> ConnectorResult<String> {
    #[cfg(windows)]
    if matches!(config.transport, Transport::NamedPipe { .. }) {
        return Err(reason_err!(
            "Redirect",
            "server redirection is not supported over a named pipe"
        ));
    }

    // The server returns the TsvUrl the client sent in its load-balancing information; on a mismatch the client must
    // disconnect ([MS-RDPBCGR] 2.2.13.1).
    // UNTESTED: a mismatching TsvUrl ends the client instead of following the redirection. It keeps a forged
    // redirection from moving a client off the collection it asked for; check it against a broker that sends
    // LB_CLIENT_TSV_URL when changing the comparison or the normalization of the load-balancing information.
    if let (Some(tsv_url), Some(load_balance_info)) = (&redirection.tsv_url, &config.load_balance_info) {
        // The TsvUrl is Unicode text, with or without a null terminator.
        let tsv_url = {
            let code_units: Vec<u16> = tsv_url
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|&code_unit| code_unit != 0)
                .collect();
            String::from_utf16_lossy(&code_units)
        };
        if tsv_url != *load_balance_info {
            return Err(reason_err!(
                "Redirect",
                "the TsvUrl of the Server Redirection PDU does not match the load-balancing information"
            ));
        }
    }

    if redirection
        .flags
        .contains(ServerRedirectionFlags::PASSWORD_IS_PK_ENCRYPTED)
        && redirection.password.is_some()
    {
        return Err(reason_err!(
            "Redirect",
            "the redirection password is encrypted for RDSTLS, which is not supported"
        ));
    }

    let load_balance_info = redirection
        .load_balance_info
        .as_deref()
        .map(routing_token)
        .transpose()?;

    let names = [
        redirection.target_fqdn.as_deref(),
        redirection.target_netbios_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|name| !name.is_empty());
    let addresses = redirection
        .target_net_address
        .iter()
        .chain(redirection.target_net_addresses.iter().flatten())
        .map(String::as_str)
        .filter(|address| !address.is_empty());

    // The transport resolves the name itself behind a gateway or an RDCleanPath proxy, whose access policies are
    // usually written against server names, so names come first there as they do in FreeRDP. A direct connection
    // prefers the addresses, like mstsc, unless `UseRedirectionServerName` asks for the name.
    // UNTESTED: this preference order. It decides which host of a farm the client reaches first and which name TLS
    // and CredSSP check; when changing it, compare with mstsc with and without `use redirection server name`, directly
    // and through a gateway.
    let is_direct = matches!(config.transport, Transport::Direct);
    let names_first = config.use_redirection_server_name || !is_direct;
    let ordered: Vec<&str> = if names_first {
        names.clone().chain(addresses.clone()).collect()
    } else {
        addresses.clone().chain(names.clone()).collect()
    };
    let mut connect_hosts: Vec<String> = Vec::new();
    for host in ordered {
        if !connect_hosts.iter().any(|known| known.eq_ignore_ascii_case(host)) {
            connect_hosts.push(host.to_owned());
        }
    }

    let port = config.destination.port();
    let target = match connect_hosts.first() {
        // Without any target, the client reconnects to the same server, which routes the connection with the
        // load-balancing information ([MS-RDPBCGR] 1.3.8).
        None => config.destination.clone(),
        // TLS and CredSSP authenticate the target by name when there is one, even when connecting to an address.
        Some(first) if is_direct => Destination::from_parts(names.clone().next().unwrap_or(first.as_str()), port),
        Some(first) => Destination::from_parts(first.clone(), port),
    };
    if is_direct {
        debug!(?connect_hosts, server_name = target.name(), "Redirection target");
    } else {
        connect_hosts.clear();
    }

    if let Some(username) = &redirection.username {
        match &mut config.connector.credentials {
            Credentials::UsernamePassword { username: current, .. } => current.clone_from(username),
            Credentials::SmartCard { .. } => debug!("Keeping the smart card identity across the redirection"),
        }
    }
    if let Some(domain) = &redirection.domain {
        config.connector.domain = (!domain.is_empty()).then(|| domain.clone());
    }
    if redirection.flags.contains(ServerRedirectionFlags::SMARTCARD_LOGON) {
        debug!("The redirecting server allows a smart card logon on the target");
    }

    let target_certificate = redirection
        .target_certificate
        .as_ref()
        .and_then(|container| container.certificate_der())
        .map(<[u8]>::to_vec);
    if redirection.target_certificate.is_some() && target_certificate.is_none() {
        warn!("The redirection target certificate container has no DER certificate");
    }

    info!(
        target = %target,
        session_id = redirection.session_id,
        has_routing_token = load_balance_info.is_some(),
        "Following server redirection"
    );

    config.destination = target;
    config.load_balance_info = load_balance_info;
    config.redirection = Some(RedirectionHop {
        connect_hosts,
        session_id: redirection.session_id,
        password_cookie: redirection.password.clone(),
        target_certificate,
    });

    return Ok(config.destination.to_string());

    /// Converts the opaque load-balancing information into an X.224 routing token, without its CR+LF terminator.
    fn routing_token(load_balance_info: &[u8]) -> ConnectorResult<String> {
        let token = load_balance_info.strip_suffix(b"\r\n").unwrap_or(load_balance_info);
        let token = token.strip_suffix(b"\0").unwrap_or(token);
        let valid = !token.is_empty()
            && token.len() <= ironrdp_pdu::nego::MAX_ROUTING_TOKEN_LENGTH
            && token.iter().all(|byte| (0x20..=0x7E).contains(byte));
        if !valid {
            return Err(reason_err!(
                "Redirect",
                "the load-balancing information of the Server Redirection PDU is not a valid routing token"
            ));
        }
        String::from_utf8(token.to_vec()).map_err(|e| ironrdp_connector::custom_err!("routing token", e))
    }
}
