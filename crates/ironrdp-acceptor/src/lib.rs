#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

use ironrdp_async::{single_sequence_step, Framed, FramedRead, FramedWrite, NetworkClient, StreamWrapper};
use ironrdp_connector::sspi::credssp::EarlyUserAuthResult;
use ironrdp_connector::sspi::{AuthIdentity, KerberosServerConfig, Username};
use ironrdp_connector::{custom_err, general_err, ConnectorResult, ServerName};
use ironrdp_core::WriteBuf;
use tracing::{debug, instrument, trace};

mod channel_connection;
mod connection;
pub mod credssp;
mod finalization;
mod util;

pub use ironrdp_connector::sspi::AuthIdentity as CredsspAuthIdentity;
pub use ironrdp_connector::DesktopSize;
use ironrdp_pdu::nego;

pub use self::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
pub use self::connection::{Acceptor, AcceptorResult, AcceptorState};
pub use self::finalization::{FinalizationSequence, FinalizationState};
use crate::credssp::resolve_generator;

pub enum BeginResult<S>
where
    S: StreamWrapper,
{
    ShouldUpgrade(S::InnerStream),
    Continue(Framed<S>),
}

pub async fn accept_begin<S>(mut framed: Framed<S>, acceptor: &mut Acceptor) -> ConnectorResult<BeginResult<S>>
where
    S: FramedRead + FramedWrite + StreamWrapper,
{
    let mut buf = WriteBuf::new();

    loop {
        if let Some(security) = acceptor.reached_security_upgrade() {
            let result = if security.is_empty() {
                BeginResult::Continue(framed)
            } else {
                BeginResult::ShouldUpgrade(framed.into_inner_no_leftover())
            };

            return Ok(result);
        }

        single_sequence_step(&mut framed, acceptor, &mut buf).await?;
    }
}

pub async fn accept_credssp<S, N>(
    framed: &mut Framed<S>,
    acceptor: &mut Acceptor,
    network_client: &mut N,
    client_computer_name: ServerName,
    public_key: Vec<u8>,
    kerberos_config: Option<KerberosServerConfig>,
) -> ConnectorResult<Option<AuthIdentity>>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
{
    let mut buf = WriteBuf::new();

    if acceptor.should_perform_credssp() {
        perform_credssp_step(
            framed,
            acceptor,
            network_client,
            &mut buf,
            client_computer_name,
            public_key,
            kerberos_config,
        )
        .await
    } else {
        Ok(None)
    }
}

pub async fn accept_finalize<S>(
    mut framed: Framed<S>,
    acceptor: &mut Acceptor,
) -> ConnectorResult<(Framed<S>, AcceptorResult)>
where
    S: FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    loop {
        if let Some(result) = acceptor.get_result() {
            return Ok((framed, result));
        }
        single_sequence_step(&mut framed, acceptor, &mut buf).await?;
    }
}

#[instrument(level = "trace", skip_all, ret)]
async fn perform_credssp_step<S, N>(
    framed: &mut Framed<S>,
    acceptor: &mut Acceptor,
    network_client: &mut N,
    buf: &mut WriteBuf,
    client_computer_name: ServerName,
    public_key: Vec<u8>,
    kerberos_config: Option<KerberosServerConfig>,
) -> ConnectorResult<Option<AuthIdentity>>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
{
    assert!(acceptor.should_perform_credssp());
    let AcceptorState::Credssp { protocol, .. } = acceptor.state else {
        unreachable!()
    };

    let result = credssp_loop(
        framed,
        acceptor,
        network_client,
        buf,
        client_computer_name,
        public_key,
        kerberos_config,
    )
    .await;

    if protocol.intersects(nego::SecurityProtocol::HYBRID_EX) {
        trace!(?result, "HYBRID_EX");

        let early_result = if result.is_ok() {
            EarlyUserAuthResult::Success
        } else {
            EarlyUserAuthResult::AccessDenied
        };

        buf.clear();
        early_result
            .to_buffer(&mut *buf)
            .map_err(|e| ironrdp_connector::custom_err!("to_buffer", e))?;
        let response = &buf[..early_result.buffer_len()];
        framed
            .write_all(response)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
    }

    let identity = result?;

    acceptor.mark_credssp_as_done();

    return Ok(identity);

    async fn credssp_loop<S, N>(
        framed: &mut Framed<S>,
        acceptor: &mut Acceptor,
        network_client: &mut N,
        buf: &mut WriteBuf,
        client_computer_name: ServerName,
        public_key: Vec<u8>,
        kerberos_config: Option<KerberosServerConfig>,
    ) -> ConnectorResult<Option<AuthIdentity>>
    where
        S: FramedRead + FramedWrite,
        N: NetworkClient,
    {
        let creds = acceptor
            .creds
            .as_ref()
            .ok_or_else(|| general_err!("no credentials while doing credssp"))?;
        let username = Username::new(&creds.username, None).map_err(|e| custom_err!("invalid username", e))?;
        let identity = AuthIdentity {
            username,
            password: creds.password.clone().into(),
        };

        let mut sequence =
            credssp::CredsspSequence::init(&identity, client_computer_name, public_key, kerberos_config)?;

        loop {
            let Some(next_pdu_hint) = sequence.next_pdu_hint()? else {
                break;
            };

            debug!(
                acceptor.state = ?acceptor.state,
                hint = ?next_pdu_hint,
                "Wait for PDU"
            );

            let pdu = framed
                .read_by_hint(next_pdu_hint)
                .await
                .map_err(|e| ironrdp_connector::custom_err!("read frame by hint", e))?;

            trace!(length = pdu.len(), "PDU received");

            let Some(ts_request) = sequence.decode_client_message(&pdu)? else {
                break;
            };

            let result = {
                let mut generator = sequence.process_ts_request(ts_request);
                resolve_generator(&mut generator, network_client).await
            }; // drop generator

            buf.clear();
            let written = sequence.handle_process_result(result, buf)?;

            if let Some(response_len) = written.size() {
                let response = &buf[..response_len];
                trace!(response_len, "Send response");
                framed
                    .write_all(response)
                    .await
                    .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
            }
        }

        Ok(sequence.take_identity())
    }
}
