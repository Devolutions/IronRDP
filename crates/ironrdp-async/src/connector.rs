use ironrdp_connector::credssp::{CredsspProcessGenerator, CredsspSequence, KerberosConfig};
use ironrdp_connector::sspi::credssp::ClientState;
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_connector::{
    custom_err, ClientConnector, ClientConnectorState, ConnectionResult, ConnectorError, ConnectorResult, Sequence,
    ServerName, State as _, Written,
};
use ironrdp_pdu::write_buf::WriteBuf;

use crate::framed::{Framed, FramedRead, FramedWrite};
use crate::{single_sequence_step, AsyncNetworkClient};

#[non_exhaustive]
pub struct ShouldUpgrade;

#[instrument(skip_all)]
pub async fn connect_begin<S>(framed: &mut Framed<S>, connector: &mut ClientConnector) -> ConnectorResult<ShouldUpgrade>
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

pub fn skip_connect_begin(connector: &mut ClientConnector) -> ShouldUpgrade {
    assert!(connector.should_perform_security_upgrade());
    ShouldUpgrade
}

#[non_exhaustive]
pub struct Upgraded;

#[instrument(skip_all)]
pub fn mark_as_upgraded(_: ShouldUpgrade, connector: &mut ClientConnector) -> Upgraded {
    trace!("Marked as upgraded");
    connector.mark_security_upgrade_as_done();
    Upgraded
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
        )
        .await?;
    }

    let result = loop {
        single_sequence_step(framed, &mut connector, &mut buf).await?;

        if let ClientConnectorState::Connected { result } = connector.state {
            break result;
        }
    };

    info!("Connected with success");

    Ok(result)
}

async fn resolve_generator(
    generator: &mut CredsspProcessGenerator<'_>,
    network_client: &mut dyn AsyncNetworkClient,
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
async fn perform_credssp_step<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    buf: &mut WriteBuf,
    server_name: ServerName,
    server_public_key: Vec<u8>,
    mut network_client: Option<&mut dyn AsyncNetworkClient>,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
{
    assert!(connector.should_perform_credssp());

    let (mut sequence, mut ts_request) =
        CredsspSequence::init(connector, server_name, server_public_key, kerberos_config)?;

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
            connector.state = connector.state.name(),
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

pub async fn single_connect_step<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    buf: &mut WriteBuf,
) -> ConnectorResult<()>
where
    S: FramedWrite + FramedRead,
{
    buf.clear();
    let written = single_connect_step_read(framed, connector, buf).await?;
    single_connect_step_write(framed, buf, written).await
}

pub async fn single_connect_step_read<S>(
    framed: &mut Framed<S>,
    connector: &mut dyn Sequence,
    buf: &mut WriteBuf,
) -> ConnectorResult<Written>
where
    S: FramedRead,
{
    buf.clear();

    if let Some(next_pdu_hint) = connector.next_pdu_hint() {
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

        connector.step(&pdu, buf)
    } else {
        connector.step_no_input(buf)
    }
}

async fn single_connect_step_write<S>(
    framed: &mut Framed<S>,
    buf: &mut WriteBuf,
    written: Written,
) -> ConnectorResult<()>
where
    S: FramedWrite,
{
    if let Some(response_len) = written.size() {
        debug_assert_eq!(buf.filled_len(), response_len);
        let response = buf.filled();
        trace!(response_len, "Send response");
        framed
            .write_all(response)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
    }

    Ok(())
}
