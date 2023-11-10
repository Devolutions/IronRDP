use std::io::{Read, Write};

use ironrdp_connector::credssp::{CredsspProcessGenerator, CredsspSequence, KerberosConfig};
use ironrdp_connector::sspi::credssp::ClientState;
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_connector::sspi::network_client::NetworkClient;
use ironrdp_connector::{
    credssp_sequence::{CredSspSequence, CredsspPCredsspSequence},
    custom_err,
    sspi::{credssp::ClientState, generator::GeneratorState, network_client::NetworkClient},
    ClientConnector, ClientConnectorState, ConnectionResult, ConnectorResult, KerberosConfig, Sequence as _,
    ServerName, State as _,
};
use ironrdp_pdu::write_buf::WriteBuf;

use crate::framed::Framed;

#[non_exhaustive]
pub struct ShouldUpgrade;

#[instrument(skip_all)]
pub fn connect_begin<S>(framed: &mut Framed<S>, connector: &mut ClientConnector) -> ConnectorResult<ShouldUpgrade>
where
    S: Sync + Read + Write,
{
    let mut buf = WriteBuf::new();

    info!("Begin connection procedure");

    while !connector.should_perform_security_upgrade() {
        single_connect_step(framed, connector, &mut buf)?;
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
pub fn connect_finalize<S>(
    _: Upgraded,
    framed: &mut Framed<S>,
    server_name: ServerName,
    server_public_key: Vec<u8>,
    network_client: &mut impl NetworkClient,
    mut connector: ClientConnector,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<ConnectionResult>
where
    S: Read + Write,
{
    let mut buf = WriteBuf::new();

    debug!("CredSSP procedure");

    if connector.should_perform_credssp() {
        perform_credssp_step(
            framed,
            &mut connector,
            &mut buf,
            server_name,
            server_public_key,
            network_client,
            kerberos_config,
        )?;
    }

    debug!("Remaining of connection sequence");

    let result = loop {
        single_connect_step(framed, &mut connector, &mut buf)?;

        if let ClientConnectorState::Connected { result } = connector.state {
            break result;
        }
    };

    info!("Connected with success");

    Ok(result)
}

#[instrument(level = "info", skip(generator, network_client))]
fn resolve_generator(
    generator: &mut CredsspProcessGenerator<'_>,
    network_client: &mut impl NetworkClient,
) -> ConnectorResult<ClientState> {
    let mut state = generator.start();
    let res = loop {
        match state {
            GeneratorState::Suspended(request) => {
                let response = network_client.send(&request).unwrap();
                state = generator.resume(Ok(response));
            }
            GeneratorState::Completed(client_state) => {
                break client_state.map_err(|e| custom_err!("failed to resolve generator", e))?
            }
        }
    };
    debug!("client state = {:?}", &res);
    Ok(res)
}

#[instrument(level = "trace", skip(network_client, framed, buf, server_name, server_public_key))]
fn perform_credssp_step<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    buf: &mut WriteBuf,
    server_name: ServerName,
    server_public_key: Vec<u8>,
    network_client: &mut impl NetworkClient,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<()>
where
    S: Read + Write,
{
    assert!(connector.should_perform_credssp());
    let mut credssp_sequence = CredsspSequence::new(connector, server_name, server_public_key, kerberos_config)?;
    while !credssp_sequence.is_done() {
        buf.clear();
        let input = if let Some(next_pdu_hint) = credssp_sequence.next_pdu_hint() {
            debug!(
                connector.state = connector.state.name(),
                hint = ?next_pdu_hint,
                "Wait for PDU"
            );

            let pdu = framed
                .read_by_hint(next_pdu_hint)
                .map_err(|e| ironrdp_connector::custom_err!("read frame by hint", e))?;

            trace!(length = pdu.len(), "PDU received");
            Some(pdu.to_vec())
        } else {
            None
        };

        if credssp_sequence.wants_request_from_server() {
            credssp_sequence.read_request_from_server(&input.unwrap_or_else(|| [].to_vec()))?;
        }
        let client_state = {
            let mut generator = credssp_sequence.process();
            resolve_generator(&mut generator, network_client)?
        }; // drop generator
        let written = credssp_sequence.handle_process_result(client_state, buf)?;

        if let Some(response_len) = written.size() {
            let response = &buf[..response_len];
            trace!(response_len, "Send response");
            framed
                .write_all(response)
                .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
        }
    }
    connector.mark_credssp_as_done();
    Ok(())
}

pub fn single_connect_step<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    buf: &mut WriteBuf,
) -> ConnectorResult<()>
where
    S: Read + Write,
{
    buf.clear();

    let written = if let Some(next_pdu_hint) = connector.next_pdu_hint() {
        debug!(
            connector.state = connector.state.name(),
            hint = ?next_pdu_hint,
            "Wait for PDU"
        );

        let pdu = framed
            .read_by_hint(next_pdu_hint)
            .map_err(|e| ironrdp_connector::custom_err!("read frame by hint", e))?;

        trace!(length = pdu.len(), "PDU received");

        connector.step(&pdu, buf)?
    } else {
        connector.step_no_input(buf)?
    };

    if let Some(response_len) = written.size() {
        let response = &buf[..response_len];
        trace!(response_len, "Send response");
        framed
            .write_all(response)
            .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
    }

    Ok(())
}
