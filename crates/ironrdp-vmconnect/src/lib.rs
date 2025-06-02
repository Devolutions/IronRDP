#[macro_use]
extern crate tracing;

use ironrdp_async::{perform_credssp_step, single_sequence_step, AsyncNetworkClient, Framed, FramedRead, FramedWrite};
use ironrdp_connector::credssp::KerberosConfig;
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectionResult, ConnectorResult, Sequence, ServerName,
};
use ironrdp_core::WriteBuf;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::pcb::PcbVersion;
use tracing::instrument;

pub fn create_pcb_payload(vm_id: &str) -> String {
    format!("{vm_id};EnhancedMode=1")
}

#[non_exhaustive]
pub struct ShouldUpgrade;

#[non_exhaustive]
pub struct Upgraded;

pub async fn connect_begin<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    vm_id: &str,
) -> ConnectorResult<ShouldUpgrade>
where
    S: Sync + FramedRead + FramedWrite,
{
    info!("Pre-connection procedure");
    let mut buf = WriteBuf::new();
    debug_assert!(matches!(
        connector.state,
        ClientConnectorState::ConnectionInitiationSendRequest
    ));

    let _ = connector.step(&[], &mut buf)?;

    let ClientConnectorState::ConnectionInitiationWaitConfirm { requested_protocol } = connector.state else {
        return Err(ironrdp_connector::reason_err!(
            "Invalid connector state",
            "Expected ConnectionInitiationWaitConfirm",
        ));
    };

    connector.state = ClientConnectorState::EnhancedSecurityUpgrade {
        selected_protocol: requested_protocol,
    };

    let pdu = ironrdp_pdu::pcb::PreconnectionBlob {
        id: 0,
        version: PcbVersion::V2,
        v2_payload: Some(create_pcb_payload(vm_id)),
    };

    let to_write = ironrdp_core::encode_vec(&pdu)
        .map_err(|e| ironrdp_connector::custom_err!("Failed to encode preconnection PDU", e))?;

    framed
        .write_all(&to_write)
        .await
        .map_err(|e| ironrdp_connector::custom_err!("Failed to write preconnection PDU", e))?;

    Ok(ShouldUpgrade)
}

#[instrument(skip_all)]
pub fn mark_as_upgraded(_: ShouldUpgrade, connector: &mut ClientConnector) -> Upgraded {
    trace!("Marked as upgraded");
    connector.mark_security_upgrade_as_done();
    Upgraded
}

#[instrument(skip_all)]
pub fn force_upgrade(_: ShouldUpgrade, connector: &mut ClientConnector, protocol: SecurityProtocol) -> Upgraded {
    trace!("Forcing security upgrade");
    connector.state = ClientConnectorState::Credssp {
        selected_protocol: protocol,
    };
    Upgraded
}

#[instrument(skip_all)]
pub fn skip_connect_begin() -> ShouldUpgrade {
    ShouldUpgrade
}

#[instrument(skip_all)]
pub async fn connect_finalize<S>(
    _: Upgraded,
    framed: &mut Framed<S>,
    mut connector: ClientConnector,
    server_name: ServerName,
    server_public_key: Vec<u8>,
    network_client: Option<&mut dyn AsyncNetworkClient>,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<ConnectionResult>
where
    S: FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    if connector.should_perform_credssp() {
        perform_credssp_step(
            framed,
            &mut connector,
            &mut buf,
            server_name,
            server_public_key,
            network_client,
            kerberos_config,
            true, // use_vmconnect
        )
        .await?;
    }

    connector.state = ClientConnectorState::ConnectionInitiationSendRequest;

    let result = loop {
        if let ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } = connector.state {
            connector.state = ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol };
        }

        if let ClientConnectorState::Connected { result } = connector.state {
            break result;
        }

        single_sequence_step(framed, &mut connector, &mut buf).await?;
    };

    info!("Connected with success");

    Ok(result)
}
