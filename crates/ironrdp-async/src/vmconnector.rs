use ironrdp_connector::{ClientConnector, ConnectorResult};
use ironrdp_pdu::pcb::PcbVersion;
use ironrdp_vmconnect::VmClientConnector;
use tracing::info;

use crate::{single_sequence_step, CredSSPFinished, Framed, FramedRead, FramedWrite};

#[non_exhaustive]
pub struct PcbSent;

pub async fn send_pcb<S>(framed: &mut Framed<S>, payload: String) -> ConnectorResult<PcbSent>
where
    S: Sync + FramedRead + FramedWrite,
{
    let pcb_pdu = ironrdp_pdu::pcb::PreconnectionBlob {
        id: 0,
        version: PcbVersion::V2,
        v2_payload: Some(payload),
    };

    let buf = ironrdp_core::encode_vec(&pcb_pdu)
        .map_err(|e| ironrdp_connector::custom_err!("encode PreconnectionBlob PDU", e))?;

    framed
        .write_all(&buf)
        .await
        .map_err(|e| ironrdp_connector::custom_err!("write PCB PDU", e))?;

    Ok(PcbSent)
}

pub fn mark_pcb_sent_by_rdclean_path() -> PcbSent {
    PcbSent
}

pub fn vm_connector_take_over(_: PcbSent, connector: ClientConnector) -> ConnectorResult<VmClientConnector> {
    VmClientConnector::take_over(connector)
}

pub async fn run_until_handover(
    credssp_finished: &mut CredSSPFinished,
    framed: &mut Framed<impl FramedRead + FramedWrite>,
    mut connector: VmClientConnector,
) -> ConnectorResult<ClientConnector> {
    let result = loop {
        single_sequence_step(framed, &mut connector, &mut credssp_finished.write_buf).await?;

        if connector.should_hand_over() {
            break connector.hand_over()?;
        }
    };

    info!("Handover to client connector");
    credssp_finished.write_buf.clear();

    Ok(result)
}
