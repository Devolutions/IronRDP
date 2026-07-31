#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

//! Hyper-V VM console (vmconnect) front-end sequencing.
//!
//! Direct Approach: **PCB → TLS → CredSSP → X.224**. This crate owns the Hyper-V-specific steps so
//! `ironrdp-connector` stays generic.
//!
//! ```text
//! stream → send_preconnection_blob → PcbSent → TLS (caller) → connect_front(PcbSent) → Upgraded
//! ```
//!
//! [`PcbSent`] is a receipt (same idea as [`ironrdp_async::ShouldUpgrade`]). TLS stays with the
//! caller. CredSSP I/O is [`ironrdp_async::perform_credssp`]. Not for RDCleanPath.

use ironrdp_async::{
    Framed, FramedRead, FramedWrite, NetworkClient, Upgraded, connect_begin, mark_as_upgraded, perform_credssp,
};
use ironrdp_connector::credssp::KerberosConfig;
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectorError, ConnectorErrorExt as _, ConnectorResult, ServerName,
    State as _, custom_err, reason_err,
};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::pcb::{PcbVersion, PreconnectionBlob};
use tracing::{debug, instrument};

/// TCP port a Hyper-V VM console listens on.
pub const PORT: u16 = 2179;

const REQUIRED_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID;

/// Receipt that the Preconnection Blob was written on the pre-TLS stream.
///
/// Only [`send_preconnection_blob`] constructs it; [`connect_front`] consumes it by value.
#[derive(Debug)]
#[must_use = "pass this to connect_front after TLS; dropping it skips the Hyper-V front"]
#[non_exhaustive]
pub struct PcbSent;

/// Send the Preconnection Blob on a **pre-TLS** stream (`vm_id` = dashed GUID). No reply.
///
/// Returns a [`PcbSent`] receipt required by [`connect_front`] after the caller upgrades to TLS.
#[instrument(skip_all, fields(%vm_id))]
pub async fn send_preconnection_blob<S>(framed: &mut Framed<S>, vm_id: &str) -> ConnectorResult<PcbSent>
where
    S: FramedWrite,
{
    let bytes = encode_vec(&PreconnectionBlob {
        id: 0,
        version: PcbVersion::V2,
        v2_payload: Some(vm_id.to_owned()),
    })
    .map_err(ConnectorError::encode)?;

    debug!(length = bytes.len(), "Send Preconnection Blob");
    framed
        .write_all(&bytes)
        .await
        .map_err(|e| custom_err!("write preconnection blob", e))?;

    Ok(PcbSent)
}

/// After TLS: CredSSP, then X.224. Consumes [`PcbSent`]; returns [`Upgraded`] for
/// [`ironrdp_async::connect_finalize`].
///
/// `connector` must be from [`ClientConnector::new`] (prefer `enable_tls` + `enable_credssp`).
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
    S: Sync + FramedRead + FramedWrite,
    N: NetworkClient,
{
    debug!("Begin NLA using CredSSP (Hyper-V Direct Approach, before X.224)");

    let mut buf = WriteBuf::new();
    perform_credssp(
        framed,
        network_client,
        &mut buf,
        connector.config.credentials.clone(),
        connector.config.domain.as_deref(),
        REQUIRED_PROTOCOL,
        server_name,
        server_public_key.to_owned(),
        kerberos_config,
    )
    .await?;

    let should_upgrade = connect_begin(framed, connector).await?;
    ensure_selected_hybrid(connector)?;

    // TLS: caller already upgraded. CredSSP: just finished above. Both marks are truthful.
    let upgraded = mark_as_upgraded(should_upgrade, connector);
    connector.mark_credssp_as_done();
    Ok(upgraded)
}

fn ensure_selected_hybrid(connector: &ClientConnector) -> ConnectorResult<()> {
    let selected = match &connector.state {
        ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } => *selected_protocol,
        other => {
            return Err(reason_err!(
                "Initiation",
                "expected EnhancedSecurityUpgrade after Hyper-V X.224 initiation, got {}",
                other.name()
            ));
        }
    };

    if selected == REQUIRED_PROTOCOL {
        Ok(())
    } else {
        Err(reason_err!(
            "Initiation",
            "server must select {REQUIRED_PROTOCOL} for a Hyper-V console, but it selected {selected}",
        ))
    }
}
