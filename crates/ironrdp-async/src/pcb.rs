use ironrdp_connector::ConnectorResult;
use ironrdp_pdu::pcb::PcbVersion;

use crate::{Framed, FramedRead, FramedWrite};

/// Proof that the preconnection blob reached the server, required to start a vmconnect sequence.
///
/// Obtained from [`send_pcb`] on a direct connection, or from [`mark_pcb_sent_by_rdcleanpath`]
/// when a gateway forwards the blob on the client's behalf.
#[non_exhaustive]
pub struct PcbSent;

/// Sends a v2 preconnection blob PDU carrying `payload` (e.g. a Hyper-V VM ID).
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

/// Marks the preconnection blob as delivered out-of-band, by an RDCleanPath request's pcb field.
pub fn mark_pcb_sent_by_rdcleanpath() -> PcbSent {
    PcbSent
}
