//! Helpers shared by the suites that test dynamic channel processors.

use ironrdp_core::encode_vec;
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};

/// Encodes the messages a dynamic channel processor returned.
pub(crate) fn encoded(messages: &[ironrdp_dvc::DvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect()
}

/// Returns the channel payloads of DRDYNVC messages, checking that each targets `channel_id`.
pub(crate) fn dvc_payloads(messages: &[ironrdp_svc::SvcMessage], channel_id: u32) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| {
            let unframed = message.encode_unframed_pdu().expect("encode DRDYNVC PDU");
            let dvc: DrdynvcClientPdu = ironrdp_core::decode(&unframed).expect("decode DRDYNVC PDU");
            let DrdynvcClientPdu::Data(data_pdu) = dvc else {
                panic!("expected DRDYNVC data");
            };
            assert_eq!(data_pdu.channel_id(), channel_id);
            let DrdynvcDataPdu::Data(data) = data_pdu else {
                panic!("expected unsplit DRDYNVC data");
            };
            data.data().to_vec()
        })
        .collect()
}
