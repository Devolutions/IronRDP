//! Expected bytes and behavior come from the Windows App 2.0.1485.0 client, the only implementation of these channels,
//! as `crates/ironrdp-w365/PROTOCOL.md` records it.

mod battery;
mod power_request;
mod screen_capture;

use crate::dvc_messages::encoded;

const CHANNEL_ID: u32 = 3;

/// Returns the channel payloads of DRDYNVC messages, checking that each targets the test channel.
fn dvc_payloads(messages: &[ironrdp_svc::SvcMessage]) -> Vec<Vec<u8>> {
    crate::dvc_messages::dvc_payloads(messages, CHANNEL_ID)
}
