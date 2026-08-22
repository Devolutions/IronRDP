#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
// The explicit unit-test target reuses this file and does not need Cargo's implicit library dependency.
#![cfg_attr(test, allow(unused_crate_dependencies))]

//! Hyper-V VM console front-end: **PCB → TLS → CredSSP → X.224**.
//!
//! ```text
//! stream → send_preconnection_blob → PcbSent → TLS (caller) → connect_front → Upgraded
//! ```
//!
//! The post-CredSSP X.224 request advertises only `HYBRID`. `HYBRID_EX` is unnecessary for
//! VMConnect and would imply an Early User Authorization Result exchange that this ordering does
//! not perform.

use core::time::Duration;

use ironrdp_async::{
    Framed, FramedRead, FramedWrite, NetworkClient, Upgraded, connect_begin, mark_as_upgraded, perform_credssp,
};
use ironrdp_connector::credssp::{CredsspSequence, KerberosConfig};
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectorError, ConnectorErrorExt as _, ConnectorResult, Credentials,
    ServerName, State as _, custom_err, reason_err,
};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::pcb::{PcbVersion, PreconnectionBlob};
use tracing::{debug, instrument};

#[cfg(windows)]
mod native_credssp;

/// TCP port a Hyper-V VM console listens on.
pub const PORT: u16 = 2179;

/// Upper bound for transmitting the Preconnection Blob after the TCP connection is established.
///
/// MS-RDPEPS requires the complete preconnection PDU within ten seconds of TCP connection creation.
/// This crate stays runtime-agnostic; async callers should enforce the deadline around
/// [`send_preconnection_blob`] (for example with `tokio::time::timeout`).
pub const PCB_TRANSMIT_DEADLINE: Duration = Duration::from_secs(10);

// CredSSP runs before X.224. The protocol value only drives our CredSSP sequence bookkeeping.
const PRE_X224_CREDSSP_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID;
const POST_CREDSSP_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID;

const ENHANCED_MODE_SUFFIX: &str = ";EnhancedMode=1";

/// Hyper-V VM console connection mode.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Use the guest's Enhanced Session RDP backend.
    #[default]
    Enhanced,
    /// Use the Hyper-V synthetic video and input console.
    Basic,
}

/// Receipt that the Preconnection Blob was written. Required by [`connect_front`].
#[derive(Debug)]
#[must_use = "pass this to a connect_front function after TLS"]
#[non_exhaustive]
pub struct PcbSent;

/// Encode a PCB V2 for the selected console mode.
pub fn encode_preconnection_blob(vm_id: &str, mode: Mode) -> ConnectorResult<Vec<u8>> {
    let payload = preconnection_blob_payload(vm_id, mode)?;
    encode_preconnection_blob_payload(payload)
}

/// Build the Unicode PCB V2 payload used to select a VM and console mode.
pub fn preconnection_blob_payload(vm_id: &str, mode: Mode) -> ConnectorResult<String> {
    if vm_id.trim().is_empty() {
        return Err(reason_err!("vmconnect", "vmconnect VM ID is empty"));
    }

    Ok(match mode {
        Mode::Enhanced => format!("{vm_id}{ENHANCED_MODE_SUFFIX}"),
        Mode::Basic => vm_id.to_owned(),
    })
}

/// Encode a PCB V2 containing an opaque routing payload.
pub fn encode_preconnection_blob_payload(payload: String) -> ConnectorResult<Vec<u8>> {
    encode_vec(&PreconnectionBlob {
        id: 0,
        version: PcbVersion::V2,
        v2_payload: Some(payload),
    })
    .map_err(ConnectorError::encode)
}

/// Write the Preconnection Blob on a pre-TLS stream. Returns a [`PcbSent`] for [`connect_front`].
///
/// Callers should bound this write with [`PCB_TRANSMIT_DEADLINE`] from TCP connection creation.
#[instrument(skip_all, fields(%vm_id, ?mode))]
pub async fn send_preconnection_blob<S>(framed: &mut Framed<S>, vm_id: &str, mode: Mode) -> ConnectorResult<PcbSent>
where
    S: FramedWrite,
{
    let bytes = encode_preconnection_blob(vm_id, mode)?;

    debug!(length = bytes.len(), "Send Preconnection Blob");
    framed
        .write_all(&bytes)
        .await
        .map_err(|e| custom_err!("write preconnection blob", e))?;

    Ok(PcbSent)
}

/// Receipt after an RDCleanPath proxy has written the PCB and established TLS.
pub fn pcb_sent_via_proxy() -> PcbSent {
    PcbSent
}

/// After TLS: CredSSP, then X.224. Consumes [`PcbSent`]; returns [`Upgraded`] for
/// [`ironrdp_async::connect_finalize`].
///
/// This path always performs CredSSP and then negotiates HYBRID. A connector that disabled TLS or
/// CredSSP is rejected so X.224 cannot advertise a protocol set that disagrees with the wire
/// sequence.
#[instrument(skip_all)]
pub async fn connect_front<S, N>(
    _pcb_sent: PcbSent,
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    network_client: &mut N,
    server_name: ServerName,
    server_public_key: &[u8],
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<Upgraded>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
{
    prepare_connector(connector)?;

    debug!("Begin CredSSP (before X.224)");

    let mut buf = WriteBuf::new();
    let (sequence, ts_request) = CredsspSequence::init(
        connector.config.credentials.clone(),
        connector.config.domain.as_deref(),
        PRE_X224_CREDSSP_PROTOCOL,
        server_name,
        server_public_key.to_owned(),
        kerberos_config,
    )?;
    perform_credssp(framed, network_client, &mut buf, sequence, ts_request).await?;

    finish_front(framed, connector, &mut buf).await
}

/// After TLS, authenticate the Hyper-V host with the caller's current Windows logon token, then
/// negotiate X.224.
///
/// This matches native VMConnect's implicit-credential path and never exposes or stores the
/// current user's password.
#[cfg(windows)]
#[instrument(skip_all)]
pub async fn connect_front_with_current_user<S>(
    _pcb_sent: PcbSent,
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    server_name: ServerName,
    server_public_key: &[u8],
) -> ConnectorResult<Upgraded>
where
    S: FramedRead + FramedWrite,
{
    prepare_connector(connector)?;

    debug!("Begin native CredSSP with current Windows credentials");
    native_credssp::perform(framed, server_name, server_public_key).await?;

    let mut buf = WriteBuf::new();
    finish_front(framed, connector, &mut buf).await
}

async fn finish_front<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    buf: &mut WriteBuf,
) -> ConnectorResult<Upgraded>
where
    S: FramedRead + FramedWrite,
{
    // Host authentication is complete. Do not forward its identity or secret into the guest-facing
    // RDP sequence; Enhanced Session guest sign-in is a separate authentication seam. This happens
    // only after successful CredSSP, so a failed authentication leaves the connector reusable.
    connector.config.credentials = Credentials::UsernamePassword {
        username: String::new(),
        password: String::new(),
    };
    connector.config.domain = None;
    connector.config.autologon = false;

    buf.clear();
    connector.initiate_with_security_protocol(POST_CREDSSP_PROTOCOL, buf)?;
    framed
        .write_all(buf.filled())
        .await
        .map_err(|e| custom_err!("write X.224 connection request", e))?;

    let should_upgrade = connect_begin(framed, connector).await?;
    ensure_selected_credssp(&connector.state)?;

    let upgraded = mark_as_upgraded(should_upgrade, connector);
    connector.mark_credssp_as_done();
    Ok(upgraded)
}

/// Require TLS + CredSSP on the connector for the VMConnect front-end.
///
/// CredSSP runs before X.224 and TLS is already up when this path is used. Clearing
/// `enable_tls` / `enable_credssp` would make the later Negotiate Request advertise a protocol
/// set that disagrees with the bytes already exchanged.
///
/// Call this before any pre-X.224 CredSSP path that does not go through [`connect_front`]
/// (for example FFI `CredsspSequence::init_with_protocol`).
pub fn prepare_connector(connector: &ClientConnector) -> ConnectorResult<()> {
    if !connector.config.enable_tls {
        return Err(reason_err!("vmconnect", "vmconnect requires TLS"));
    }
    if !connector.config.enable_credssp {
        return Err(reason_err!("vmconnect", "vmconnect requires CredSSP"));
    }
    Ok(())
}

/// Require the post-CredSSP X.224 response to select HYBRID.
pub fn ensure_selected_credssp(state: &ClientConnectorState) -> ConnectorResult<()> {
    let selected = match state {
        ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } => *selected_protocol,
        other => {
            return Err(reason_err!(
                "Initiation",
                "expected EnhancedSecurityUpgrade after Hyper-V X.224 initiation, got {}",
                other.name()
            ));
        }
    };

    if selected == SecurityProtocol::HYBRID {
        Ok(())
    } else {
        Err(reason_err!(
            "Initiation",
            "server must select HYBRID for a Hyper-V console, but it selected {selected}",
        ))
    }
}
