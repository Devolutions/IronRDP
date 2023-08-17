#[macro_use]
extern crate tracing;

use ironrdp_async::{Framed, FramedRead, FramedWrite, StreamWrapper};
use ironrdp_connector::{custom_err, ConnectorResult, Sequence, Written};

mod channel_connection;
mod connection;
mod finalization;
mod util;

pub use connection::{Acceptor, AcceptorResult};
pub use ironrdp_connector::DesktopSize;

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
    let mut buf = Vec::new();

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
    let mut buf = Vec::new();

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
    buf: &mut Vec<u8>,
) -> ConnectorResult<Written>
where
    S: FramedRead + FramedWrite,
{
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

    if let Some(len) = written.size() {
        trace!(length = len, "Send response");
        framed
            .write_all(&buf[..len])
            .await
            .map_err(|e| custom_err!("write all", e))?;
    }

    Ok(written)
}
