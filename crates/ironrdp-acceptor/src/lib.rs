#[macro_use]
extern crate tracing;

use ironrdp_async::{Framed, FramedRead, FramedWrite, StreamWrapper};
use ironrdp_connector::{custom_err, ConnectorResult, Sequence, Written};
use ironrdp_pdu::write_buf::WriteBuf;

mod channel_connection;
mod connection;
mod finalization;
mod util;

pub use ironrdp_connector::DesktopSize;

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

        single_accept_state(&mut framed, acceptor, &mut buf).await?;
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

        single_accept_state(&mut framed, acceptor, &mut buf).await?;
    }
}

async fn single_accept_state<S>(
    framed: &mut Framed<S>,
    acceptor: &mut Acceptor,
    buf: &mut WriteBuf,
) -> ConnectorResult<Written>
where
    S: FramedRead + FramedWrite,
{
    buf.clear();

    let written = if let Some(next_pdu_hint) = acceptor.next_pdu_hint() {
        debug!(
            acceptor.state = acceptor.state().name(),
            hint = ?next_pdu_hint,
            "Wait for PDU"
        );

        let pdu = framed
            .read_by_hint(next_pdu_hint)
            .await
            .map_err(|e| custom_err!("read frame by hint", e))?;

        trace!(length = pdu.len(), "PDU received");

        acceptor.step(&pdu, buf)?
    } else {
        acceptor.step_no_input(buf)?
    };

    if let Some(response_len) = written.size() {
        debug_assert_eq!(buf.filled_len(), response_len);
        let response = buf.filled();
        trace!(response_len, "Send response");
        framed
            .write_all(response)
            .await
            .map_err(|e| custom_err!("write all", e))?;
    }

    Ok(written)
}
