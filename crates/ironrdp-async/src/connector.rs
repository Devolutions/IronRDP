use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectionResult, ConnectorResult, Sequence as _, State as _,
};
use ironrdp_pdu::write_buf::WriteBuf;

use crate::framed::{Framed, FramedRead, FramedWrite};

pub struct ShouldUpgrade {
    _priv: (),
}

#[instrument(skip_all)]
pub async fn connect_begin<S>(framed: &mut Framed<S>, connector: &mut ClientConnector) -> ConnectorResult<ShouldUpgrade>
where
    S: Sync + FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    info!("Begin connection procedure");

    while !connector.should_perform_security_upgrade() {
        single_connect_step(framed, connector, &mut buf).await?;
    }

    Ok(ShouldUpgrade { _priv: () })
}

pub fn skip_connect_begin(connector: &mut ClientConnector) -> ShouldUpgrade {
    assert!(connector.should_perform_security_upgrade());
    ShouldUpgrade { _priv: () }
}

pub struct Upgraded {
    _priv: (),
}

#[instrument(skip_all)]
pub fn mark_as_upgraded(_: ShouldUpgrade, connector: &mut ClientConnector, server_public_key: Vec<u8>) -> Upgraded {
    trace!("marked as upgraded");
    connector.attach_server_public_key(server_public_key);
    connector.mark_security_upgrade_as_done();
    Upgraded { _priv: () }
}

#[instrument(skip_all)]
pub async fn connect_finalize<S>(
    _: Upgraded,
    framed: &mut Framed<S>,
    mut connector: ClientConnector,
) -> ConnectorResult<ConnectionResult>
where
    S: FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    debug!("CredSSP procedure");

    while connector.is_credssp_step() {
        single_connect_step(framed, &mut connector, &mut buf).await?;
    }

    debug!("Remaining of connection sequence");

    let result = loop {
        single_connect_step(framed, &mut connector, &mut buf).await?;

        if let ClientConnectorState::Connected { result } = connector.state {
            break result;
        }
    };

    info!("Connected with success");

    Ok(result)
}

pub async fn single_connect_step<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    buf: &mut WriteBuf,
) -> ConnectorResult<ironrdp_connector::Written>
where
    S: FramedWrite + FramedRead,
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
            .await
            .map_err(|e| ironrdp_connector::custom_err!("read frame by hint", e))?;

        trace!(length = pdu.len(), "PDU received");

        connector.step(&pdu, buf)?
    } else {
        connector.step_no_input(buf)?
    };

    if let Some(response_len) = written.size() {
        debug_assert_eq!(buf.filled_len(), response_len);
        let response = buf.filled();
        trace!(response_len, "Send response");
        framed
            .write_all(response)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
    }

    Ok(written)
}
