#[macro_use]
extern crate tracing;

use ironrdp_async::bytes::Bytes;
use ironrdp_async::{single_sequence_step, Framed, FramedRead, FramedWrite, StreamWrapper};
use ironrdp_connector::credssp::KerberosConfig;
use ironrdp_connector::sspi::credssp::EarlyUserAuthResult;
use ironrdp_connector::sspi::{AuthIdentity, Username};
use ironrdp_connector::{custom_err, ConnectorResult, ServerName};
use ironrdp_core::WriteBuf;

mod channel_connection;
mod connection;
mod credssp;
mod finalization;
mod util;

pub use ironrdp_connector::DesktopSize;
use ironrdp_pdu::nego;

pub use self::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
pub use self::connection::{Acceptor, AcceptorResult, AcceptorState};
pub use self::finalization::{FinalizationSequence, FinalizationState};

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

        single_sequence_step(&mut framed, acceptor, &mut buf, None).await?;
    }
}

pub async fn accept_credssp<S>(
    framed: &mut Framed<S>,
    acceptor: &mut Acceptor,
    client_computer_name: ServerName,
    public_key: Vec<u8>,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    if acceptor.should_perform_credssp() {
        perform_credssp_step(
            framed,
            acceptor,
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
    mut framed: Framed<S>,
    acceptor: &mut Acceptor,
    mut unmatched: Option<&mut Vec<Bytes>>,
) -> ConnectorResult<(Framed<S>, AcceptorResult)>
where
    S: FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    loop {
        if let Some(result) = acceptor.get_result() {
            return Ok((framed, result));
        }
        single_sequence_step(&mut framed, acceptor, &mut buf, unmatched.as_deref_mut()).await?;
    }
}

#[instrument(level = "trace", skip_all, ret)]
async fn perform_credssp_step<S>(
    framed: &mut Framed<S>,
    acceptor: &mut Acceptor,
    buf: &mut WriteBuf,
    client_computer_name: ServerName,
    public_key: Vec<u8>,
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
{
    assert!(acceptor.should_perform_credssp());
    let AcceptorState::Credssp { protocol, .. } = acceptor.state else {
        unreachable!()
    };

    async fn credssp_loop<S>(
        framed: &mut Framed<S>,
        acceptor: &mut Acceptor,
        buf: &mut WriteBuf,
        client_computer_name: ServerName,
        public_key: Vec<u8>,
        kerberos_config: Option<KerberosConfig>,
    ) -> ConnectorResult<()>
    where
        S: FramedRead + FramedWrite,
    {
        let creds = acceptor.creds.as_ref().unwrap();
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
                .read_by_hint(next_pdu_hint, None)
                .await
                .map_err(|e| ironrdp_connector::custom_err!("read frame by hint", e))?;

            trace!(length = pdu.len(), "PDU received");

            let Some(ts_request) = sequence.decode_client_message(&pdu)? else {
                break;
            };

            let result = sequence.process_ts_request(ts_request);
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

    let result = match credssp_loop(framed, acceptor, buf, client_computer_name, public_key, kerberos_config).await {
        Ok(_) => EarlyUserAuthResult::Success,
        Err(err) => {
            warn!("credssp: {err}");
            EarlyUserAuthResult::AccessDenied
        }
    };

    if protocol.intersects(nego::SecurityProtocol::HYBRID_EX) {
        trace!(?result, "HYBRID_EX");
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
    acceptor.mark_credssp_as_done();

    Ok(())
}
