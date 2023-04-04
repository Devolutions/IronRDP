use ironrdp_connector::{ClientConnector, ClientConnectorState, ConnectionResult, Sequence as _, State as _};

use crate::framed::{Framed, FramedRead, FramedWrite};

pub struct ShouldUpgrade {
    _priv: (),
}

#[instrument(skip_all)]
pub async fn connect_begin<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
) -> ironrdp_connector::Result<ShouldUpgrade>
where
    S: Sync + FramedRead + FramedWrite,
{
    let mut buf = Vec::new();

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
) -> ironrdp_connector::Result<ConnectionResult>
where
    S: FramedRead + FramedWrite,
{
    let mut buf = Vec::new();

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
    buf: &mut Vec<u8>,
) -> ironrdp_connector::Result<ironrdp_connector::Written>
where
    S: FramedWrite + FramedRead,
{
    let written = if let Some(next_pdu_hint) = connector.next_pdu_hint() {
        debug!(
            connector.state = connector.state.name(),
            hint = ?next_pdu_hint,
            "Wait for PDU"
        );

        let pdu = framed
            .read_by_hint(next_pdu_hint)
            .await
            .map_err(|e| ironrdp_connector::Error::new("read frame by hint").with_custom(e))?;

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
            .await
            .map_err(|e| ironrdp_connector::Error::new("write all").with_custom(e))?;
    }

    Ok(written)
}
