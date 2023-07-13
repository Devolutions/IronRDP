use std::io;

use ironrdp_connector::{custom_err, ConnectorResult, Sequence, Written};
use ironrdp_tokio::{Framed, FramedRead, FramedWrite, TokioFramed};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;

use self::connection::AcceptorResult;
pub use self::connection::ServerAcceptor;

use super::RdpServerSecurity;
mod channel_connection;
mod connection;
mod finalization;
mod util;

pub enum BeginResult<S> {
    ShouldUpgrade(S),
    Continue(TokioFramed<S>),
}

pub async fn accept_begin<S>(stream: S, acceptor: &mut ServerAcceptor) -> ConnectorResult<BeginResult<S>>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let mut buf = Vec::new();
    let mut framed = TokioFramed::new(stream);

    loop {
        if let Some(security) = acceptor.reached_security_upgrade() {
            let result = if security.is_empty() {
                BeginResult::Continue(framed)
            } else {
                BeginResult::ShouldUpgrade(framed.into_inner().0)
            };

            return Ok(result);
        }

        single_accept_state(&mut framed, acceptor, &mut buf).await?;
    }
}

pub async fn upgrade<S>(security: &RdpServerSecurity, stream: S) -> Result<TlsStream<S>, io::Error>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    match security {
        RdpServerSecurity::None => unreachable!(),
        RdpServerSecurity::SSL(tls_acceptor) => tls_acceptor.accept(stream).await,
    }
}

pub async fn accept_finalize<S>(
    mut framed: Framed<S>,
    acceptor: &mut ServerAcceptor,
) -> ConnectorResult<(Framed<S>, AcceptorResult)>
where
    S: FramedWrite + FramedRead,
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
    acceptor: &mut ServerAcceptor,
    buf: &mut Vec<u8>,
) -> ConnectorResult<Written>
where
    S: FramedWrite + FramedRead,
{
    let written = if let Some(next_pdu_hint) = acceptor.next_pdu_hint() {
        let pdu = framed
            .read_by_hint(next_pdu_hint)
            .await
            .map_err(|e| custom_err!("read frame by hint", e))?;

        acceptor.step(&pdu, buf)?
    } else {
        acceptor.step_no_input(buf)?
    };

    if let Some(len) = written.size() {
        framed
            .write_all(&buf[..len])
            .await
            .map_err(|e| custom_err!("write all", e))?;
    }

    Ok(written)
}
