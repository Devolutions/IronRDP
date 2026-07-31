#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

//! Hyper-V VM console front-end: **PCB → TLS → CredSSP → X.224**.
//!
//! ```text
//! stream → send_preconnection_blob → PcbSent → TLS (caller) → connect_front → Upgraded
//! ```

use ironrdp_async::{
    Framed, FramedRead, FramedWrite, NetworkClient, Upgraded, connect_begin, mark_as_upgraded, perform_credssp,
};
use ironrdp_connector::credssp::{CredsspSequence, KerberosConfig};
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

// CredSSP runs before X.224, so HYBRID_EX cannot request an Early User Authorization Result yet.
const PRE_X224_CREDSSP_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID;

/// PCB V2 payload always requests the enhanced RDP console stack.
const ENHANCED_MODE_SUFFIX: &str = ";EnhancedMode=1";

/// Receipt that the Preconnection Blob was written. Required by [`connect_front`].
#[derive(Debug)]
#[must_use = "pass this to connect_front after TLS"]
#[non_exhaustive]
pub struct PcbSent;

/// Encode PCB V2 as `{vm_id};EnhancedMode=1`.
pub fn encode_preconnection_blob(vm_id: &str) -> ConnectorResult<Vec<u8>> {
    let payload = format!("{vm_id}{ENHANCED_MODE_SUFFIX}");
    encode_vec(&PreconnectionBlob {
        id: 0,
        version: PcbVersion::V2,
        v2_payload: Some(payload),
    })
    .map_err(ConnectorError::encode)
}

/// Write the Preconnection Blob on a pre-TLS stream. Returns a [`PcbSent`] for [`connect_front`].
#[instrument(skip_all, fields(%vm_id))]
pub async fn send_preconnection_blob<S>(framed: &mut Framed<S>, vm_id: &str) -> ConnectorResult<PcbSent>
where
    S: FramedWrite,
{
    let bytes = encode_preconnection_blob(vm_id)?;

    debug!(length = bytes.len(), "Send Preconnection Blob");
    framed
        .write_all(&bytes)
        .await
        .map_err(|e| custom_err!("write preconnection blob", e))?;

    Ok(PcbSent)
}

/// After TLS: CredSSP, then X.224. Consumes [`PcbSent`]; returns [`Upgraded`] for
/// [`ironrdp_async::connect_finalize`].
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

    let should_upgrade = connect_begin(framed, connector).await?;
    ensure_selected_credssp(&connector.state)?;

    let upgraded = mark_as_upgraded(should_upgrade, connector);
    connector.mark_credssp_as_done();
    Ok(upgraded)
}

fn ensure_selected_credssp(state: &ClientConnectorState) -> ConnectorResult<()> {
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

    if selected == SecurityProtocol::HYBRID || selected == SecurityProtocol::HYBRID_EX {
        Ok(())
    } else {
        Err(reason_err!(
            "Initiation",
            "server must select HYBRID or HYBRID_EX for a Hyper-V console, but it selected {selected}",
        ))
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_connector::ClientConnectorState;
    use ironrdp_core::decode;
    use ironrdp_pdu::nego::SecurityProtocol;
    use ironrdp_pdu::pcb::PreconnectionBlob;

    use super::{encode_preconnection_blob, ensure_selected_credssp};

    #[test]
    fn pcb_payload_always_requests_enhanced_mode() {
        const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";

        let bytes = encode_preconnection_blob(VM_ID).expect("encode");
        let pcb: PreconnectionBlob = decode(&bytes).expect("decode");

        assert_eq!(
            pcb.v2_payload.as_deref(),
            Some("efd1efab-c750-4262-b1bb-af0f7733bdd6;EnhancedMode=1")
        );
    }

    #[test]
    fn accepts_credssp_protocol_selected_after_enhanced_mode_authentication() {
        for selected_protocol in [SecurityProtocol::HYBRID, SecurityProtocol::HYBRID_EX] {
            let state = ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol };

            ensure_selected_credssp(&state).expect("CredSSP protocol");
        }
    }
}
