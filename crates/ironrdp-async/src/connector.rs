use ironrdp_connector::credssp::{CredsspProcessGenerator, KerberosConfig};
use ironrdp_connector::sspi::credssp::ClientState;
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_connector::{
    custom_err, general_err, ClientConnector, ClientConnectorState, ConnectionResult, ConnectorCore, ConnectorError,
    ConnectorResult, SecurityConnector, ServerName,
};
use ironrdp_core::WriteBuf;
use tracing::{debug, info, instrument, trace};

use crate::framed::{Framed, FramedRead, FramedWrite};
use crate::{single_sequence_step, NetworkClient};

#[non_exhaustive]
pub struct ShouldUpgrade;

#[instrument(skip_all)]
pub async fn connect_begin<S>(
    framed: &mut Framed<S>,
    connector: &mut dyn ConnectorCore,
) -> ConnectorResult<ShouldUpgrade>
where
    S: Sync + FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    info!("Begin connection procedure");

    while !connector.should_perform_security_upgrade() {
        single_sequence_step(framed, connector, &mut buf).await?;
    }

    Ok(ShouldUpgrade)
}

/// # Panics
///
/// Panics if connector state is not [ClientConnectorState::EnhancedSecurityUpgrade].
pub fn skip_connect_begin(connector: &mut dyn SecurityConnector) -> ShouldUpgrade {
    assert!(connector.should_perform_security_upgrade());
    ShouldUpgrade
}

#[non_exhaustive]
pub struct Upgraded;

#[instrument(skip_all)]
pub fn mark_as_upgraded(_: ShouldUpgrade, connector: &mut dyn SecurityConnector) -> Upgraded {
    trace!("Marked as upgraded");
    connector.mark_security_upgrade_as_done();
    Upgraded
}

#[non_exhaustive]
pub struct CredSSPFinished {
    pub(crate) write_buf: WriteBuf,
}

#[instrument(skip_all)]
pub async fn perform_credssp<S, N>(
    _: Upgraded,
    connector: &mut dyn ConnectorCore,
    framed: &mut Framed<S>,
    server_name: ServerName,
    server_public_key: Vec<u8>,
    network_client: Option<&mut N>,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<CredSSPFinished>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
{
    let mut buf = WriteBuf::new();

    if connector.should_perform_credssp() {
        perform_credssp_step(
            connector,
            framed,
            network_client,
            &mut buf,
            server_name,
            server_public_key,
            kerberos_config,
        )
        .await?;
    }

    Ok(CredSSPFinished { write_buf: buf })
}

#[instrument(skip_all)]
pub async fn connect_finalize<S>(
    CredSSPFinished { write_buf: mut buf }: CredSSPFinished,
    framed: &mut Framed<S>,
    mut connector: ClientConnector,
) -> ConnectorResult<ConnectionResult>
where
    S: FramedRead + FramedWrite,
{
    buf.clear();
    let result = loop {
        single_sequence_step(framed, &mut connector, &mut buf).await?;

        if let ClientConnectorState::Connected { result } = connector.state {
            break result;
        }
    };

    info!("Connected with success");

    Ok(result)
}

async fn resolve_generator<N: NetworkClient>(
    generator: &mut CredsspProcessGenerator<'_>,
    network_client: &mut N,
) -> ConnectorResult<ClientState> {
    let mut state = generator.start();

    loop {
        match state {
            GeneratorState::Suspended(request) => {
                let response = network_client.send(&request).await?;
                state = generator.resume(Ok(response));
            }
            GeneratorState::Completed(client_state) => {
                break client_state
                    .map_err(|e| ConnectorError::new("CredSSP", ironrdp_connector::ConnectorErrorKind::Credssp(e)))
            }
        }
    }
}

#[instrument(level = "trace", skip_all)]
async fn perform_credssp_step<S, N>(
    connector: &mut dyn ConnectorCore,
    framed: &mut Framed<S>,
    mut network_client: Option<&mut N>,
    buf: &mut WriteBuf,
    server_name: ServerName,
    server_public_key: Vec<u8>,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
{
    assert!(connector.should_perform_credssp());

    let selected_protocol = connector
        .selected_protocol()
        .ok_or_else(|| general_err!("CredSSP protocol not selected, cannot perform CredSSP step"))?;

    let (mut sequence, mut ts_request) = connector.init_credssp(
        connector.config().credentials.clone(),
        connector.config().domain.as_deref(),
        selected_protocol,
        server_name,
        server_public_key,
        kerberos_config,
    )?;

    loop {
        let client_state = {
            let mut generator = sequence.process_ts_request(ts_request);

            if let Some(network_client_ref) = network_client.as_deref_mut() {
                trace!("resolving network");
                resolve_generator(&mut generator, network_client_ref).await?
            } else {
                generator
                    .resolve_to_result()
                    .map_err(|e| custom_err!("resolve without network client", e))?
            }
        }; // drop generator

        buf.clear();
        let written = sequence.handle_process_result(client_state, buf)?;

        if let Some(response_len) = written.size() {
            let response = &buf[..response_len];
            trace!(response_len, "Send response");
            framed
                .write_all(response)
                .await
                .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
        }

        let Some(next_pdu_hint) = sequence.next_pdu_hint() else {
            break;
        };

        debug!(
            connector.state = connector.state().name(),
            hint = ?next_pdu_hint,
            "Wait for PDU"
        );

        let pdu = framed
            .read_by_hint(next_pdu_hint)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("read frame by hint", e))?;

        trace!(length = pdu.len(), "PDU received");

        if let Some(next_request) = sequence.decode_server_message(&pdu)? {
            ts_request = next_request;
        } else {
            break;
        }
    }

    connector.mark_credssp_as_done();

    Ok(())
}
