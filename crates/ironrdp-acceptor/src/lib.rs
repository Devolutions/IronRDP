#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

use ironrdp_async::{Framed, FramedRead, FramedWrite, NetworkClient, StreamWrapper, single_sequence_step};
use ironrdp_connector::sspi::credssp::EarlyUserAuthResult;
use ironrdp_connector::sspi::{AuthIdentity, KerberosServerConfig, Username};
use ironrdp_connector::{ConnectorResult, ServerName, custom_err, general_err};
use ironrdp_core::WriteBuf;
use tracing::{debug, instrument, trace};

mod channel_connection;
mod connection;
pub mod credssp;
mod finalization;
mod util;

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
) -> ConnectorResult<()>
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
    accept_finalize_with_multitransport(framed, acceptor, |_, _| async {}).await
}

/// Completes the connection sequence, notifying `multitransport_handler` each
/// time the acceptor sends an Initiate Multitransport Request, so the caller
/// can establish the sideband UDP transport (RDPEUDP2 + TLS + RDPEMT).
///
/// Unlike [`ironrdp_async::connect_finalize_with_multitransport`] on the
/// client side, `multitransport_handler` does not report an outcome back
/// into the sequence: by the time it runs, the acceptor has already sent the
/// request and moved on to capability negotiation (see the doc comment on
/// [`ironrdp_acceptor::AcceptorState::MultitransportBootstrapping`]), and
/// nothing here waits for the sideband transport to come up. Establishing it
/// is the caller's job, driven independently of this function's return.
/// `multitransport_handler` is awaited once per request, synchronously,
/// immediately after the sequence step that sent it: it should return
/// promptly (for example, by spawning the actual UDP-accept work on the
/// caller's own runtime) rather than driving the transport to completion
/// inline, or the RDP handshake stalls behind it.
///
/// # Panics
///
/// Panics if `multitransport_soft_sync_negotiated()` returns `None` right
/// after `multitransport_request()` returned `Some`, which the two methods'
/// own contract does not allow.
pub async fn accept_finalize_with_multitransport<S, H>(
    mut framed: Framed<S>,
    acceptor: &mut Acceptor,
    mut multitransport_handler: H,
) -> ConnectorResult<(Framed<S>, AcceptorResult)>
where
    S: FramedRead + FramedWrite,
    H: AsyncFnMut(ironrdp_pdu::rdp::multitransport::MultitransportRequestPdu, bool),
{
    let mut buf = WriteBuf::new();
    // `multitransport_request()` borrows rather than consumes (the field it
    // reads also gates `CapabilitiesWaitConfirm`'s tolerance for a late
    // Initiate Multitransport Response), so this driver tracks locally
    // whether it has already notified the caller instead. Seeded from
    // whatever is already present rather than `false`: a Deactivation-
    // Reactivation Sequence rebuilds the acceptor via
    // `Acceptor::new_deactivation_reactivation()`, which carries the
    // original request forward, then this function is called again on the
    // rebuilt acceptor. Bootstrapping does not run a second time, so without
    // this the first loop iteration of that fresh call would treat the
    // carried-over request as newly sent and notify the handler again.
    let mut notified = acceptor.multitransport_request().is_some();

    loop {
        if let Some(result) = acceptor.get_result() {
            return Ok((framed, result));
        }

        single_sequence_step(&mut framed, acceptor, &mut buf).await?;

        if !notified && let Some(request) = acceptor.multitransport_request() {
            let request = request.clone();
            let soft_sync = acceptor
                .multitransport_soft_sync_negotiated()
                .expect("multitransport_request() just returned Some, so a request was sent");
            multitransport_handler(request, soft_sync).await;
            notified = true;
        }
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
) -> ConnectorResult<()>
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

        let result = if result.is_ok() {
            EarlyUserAuthResult::Success
        } else {
            EarlyUserAuthResult::AccessDenied
        };

        buf.clear();
        result
            .to_buffer(&mut *buf)
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

        Ok(())
    }
}
