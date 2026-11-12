//! Expected bytes and behavior come from the Windows App 2.0.1485.0 client, the only implementation of these channels,
//! as `crates/ironrdp-w365/PROTOCOL.md` records it.

mod battery;
mod power_request;
mod screen_capture;
mod vdcswitch;

use ironrdp_core::encode_vec;
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};

const CHANNEL_ID: u32 = 3;

/// Encodes the messages a dynamic channel processor returned.
fn encoded(messages: &[ironrdp_dvc::DvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect()
}

/// Returns the channel payloads of DRDYNVC messages, checking that each targets the test channel.
fn dvc_payloads(messages: &[ironrdp_svc::SvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| {
            let unframed = message.encode_unframed_pdu().expect("encode DRDYNVC PDU");
            let dvc: DrdynvcClientPdu = ironrdp_core::decode(&unframed).expect("decode DRDYNVC PDU");
            let DrdynvcClientPdu::Data(data_pdu) = dvc else {
                panic!("expected DRDYNVC data");
            };
            assert_eq!(data_pdu.channel_id(), CHANNEL_ID);
            let DrdynvcDataPdu::Data(data) = data_pdu else {
                panic!("expected unsplit DRDYNVC data");
            };
            data.data().to_vec()
        })
        .collect()
}
