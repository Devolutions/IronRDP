#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

use ironrdp_async::{Framed, FramedRead, FramedWrite, NetworkClient, StreamWrapper, single_sequence_step};
use ironrdp_connector::sspi::credssp::EarlyUserAuthResult;
use ironrdp_connector::sspi::{AuthIdentity, KerberosServerConfig, Username};
use ironrdp_connector::{ServerName, custom_err, general_err};
use ironrdp_core::WriteBuf;
use tracing::{debug, instrument, trace};

mod channel_connection;
mod connection;
pub mod credssp;
mod finalization;
mod util;

pub use ironrdp_connector::{ConnectorError, ConnectorErrorExt, ConnectorResult, DesktopSize};
use ironrdp_pdu::nego;

pub use self::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
pub use self::connection::{Acceptor, AcceptorResult, AcceptorState, CredentialOrigin, ReceivedCredentials};
pub use self::finalization::{FinalizationSequence, FinalizationState};
use crate::credssp::resolve_generator;

pub enum BeginResult<S>
where
    S: StreamWrapper,
{
    ShouldUpgrade(S::InnerStream),
    Continue(Framed<S>),
}

/// Async hooks that complete authenticated connection setup at protocol-safe points.
///
/// Credential handling runs before HYBRID_EX reports success. Capability setup
/// runs after the operational desktop size is known but before capability exchange.
#[async_trait::async_trait(?Send)]
pub trait ConnectionSetupHandler {
    async fn handle_credentials(&mut self, credentials: Option<ReceivedCredentials>) -> ConnectorResult<()>;

    async fn prepare_capability_exchange(&mut self, desktop_size: DesktopSize) -> ConnectorResult<()> {
        let _ = desktop_size;
        Ok(())
    }
}

struct NoopConnectionSetupHandler;

#[async_trait::async_trait(?Send)]
impl ConnectionSetupHandler for NoopConnectionSetupHandler {
    async fn handle_credentials(&mut self, _credentials: Option<ReceivedCredentials>) -> ConnectorResult<()> {
        Ok(())
    }
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
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
{
    accept_credssp_with(
        framed,
        acceptor,
        network_client,
        client_computer_name,
        public_key,
        kerberos_config,
        &mut NoopConnectionSetupHandler,
    )
    .await
}

/// Runs CredSSP and invokes `connection_setup_handler` before HYBRID_EX reports success.
pub async fn accept_credssp_with<S, N, H>(
    framed: &mut Framed<S>,
    acceptor: &mut Acceptor,
    network_client: &mut N,
    client_computer_name: ServerName,
    public_key: Vec<u8>,
    kerberos_config: Option<KerberosServerConfig>,
    credentials_handler: &mut H,
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
    H: ConnectionSetupHandler,
{
    if acceptor.should_perform_credssp() {
        perform_credssp_step(
            framed,
            acceptor,
            network_client,
            client_computer_name,
            public_key,
            kerberos_config,
            credentials_handler,
        )
        .await
    } else {
        Ok(())
    }
}

pub async fn accept_finalize<S>(
    framed: Framed<S>,
    acceptor: &mut Acceptor,
) -> ConnectorResult<(Framed<S>, AcceptorResult)>
where
    S: FramedRead + FramedWrite,
{
    accept_finalize_with(framed, acceptor, &mut NoopConnectionSetupHandler).await
}

/// Finalizes the RDP handshake and invokes `connection_setup_handler` before capability exchange.
pub async fn accept_finalize_with<S, H>(
    mut framed: Framed<S>,
    acceptor: &mut Acceptor,
    credentials_handler: &mut H,
) -> ConnectorResult<(Framed<S>, AcceptorResult)>
where
    S: FramedRead + FramedWrite,
    H: ConnectionSetupHandler,
{
    let mut buf = WriteBuf::new();

    loop {
        if let Some(result) = acceptor.get_result() {
            return Ok((framed, result));
        }
        single_sequence_step(&mut framed, acceptor, &mut buf).await?;
        if acceptor.credentials_need_handling() {
            if let Err(error) = credentials_handler
                .handle_credentials(acceptor.received_credentials().cloned())
                .await
            {
                buf.clear();
                let written = acceptor.encode_access_denied(&mut buf)?;
                framed
                    .write_all(&buf[..written])
                    .await
                    .map_err(|e| ironrdp_connector::custom_err!("write access denied", e))?;
                return Err(error);
            }
            acceptor.mark_credentials_handled();
        }
        if !acceptor.is_reactivation()
            && !acceptor.is_auto_reconnect_attempt()
            && acceptor.is_ready_for_capability_exchange()
            && let Err(error) = credentials_handler
                .prepare_capability_exchange(acceptor.desktop_size())
                .await
        {
            buf.clear();
            let written = acceptor.encode_access_denied(&mut buf)?;
            framed
                .write_all(&buf[..written])
                .await
                .map_err(|e| ironrdp_connector::custom_err!("write access denied", e))?;
            return Err(error);
        }
    }
}

#[instrument(level = "trace", skip_all, ret)]
async fn perform_credssp_step<S, N, H>(
    framed: &mut Framed<S>,
    acceptor: &mut Acceptor,
    network_client: &mut N,
    client_computer_name: ServerName,
    public_key: Vec<u8>,
    kerberos_config: Option<KerberosServerConfig>,
    credentials_handler: &mut H,
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
    N: NetworkClient,
    H: ConnectionSetupHandler,
{
    assert!(acceptor.should_perform_credssp());
    let mut buf = WriteBuf::new();
    let AcceptorState::Credssp { protocol, .. } = acceptor.state else {
        unreachable!()
    };

    let result = credssp_loop(
        framed,
        acceptor,
        network_client,
        &mut buf,
        client_computer_name,
        public_key,
        kerberos_config,
    )
    .await;

    let result = match result {
        Ok(()) => match credentials_handler
            .handle_credentials(acceptor.received_credentials().cloned())
            .await
        {
            Ok(()) => {
                acceptor.mark_credentials_handled();
                Ok(())
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };

    if protocol.intersects(nego::SecurityProtocol::HYBRID_EX) {
        trace!(?result, "HYBRID_EX");

        let result = if result.is_ok() {
            EarlyUserAuthResult::Success
        } else {
            EarlyUserAuthResult::AccessDenied
        };

        buf.clear();
        result
            .to_buffer(&mut buf)
            .map_err(|e| ironrdp_connector::custom_err!("to_buffer", e))?;
        let response = &buf[..result.buffer_len()];
        framed
            .write_all(response)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
    }

    result?;

    acceptor.mark_credssp_as_done();

    return Ok(());

    async fn credssp_loop<S, N>(
        framed: &mut Framed<S>,
        acceptor: &mut Acceptor,
        network_client: &mut N,
        buf: &mut WriteBuf,
        client_computer_name: ServerName,
        public_key: Vec<u8>,
        kerberos_config: Option<KerberosServerConfig>,
    ) -> ConnectorResult<()>
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
            let (written, delegated_credentials) = sequence.handle_process_result(result, buf)?;
            if let Some(credentials) = delegated_credentials {
                acceptor.set_received_credssp_credentials(credentials);
            }

            if let Some(response_len) = written.size() {
                let response = &buf[..response_len];
                trace!(response_len, "Send response");
                framed
                    .write_all(response)
                    .await
                    .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
            }
        }

        Ok(())
    }
}
