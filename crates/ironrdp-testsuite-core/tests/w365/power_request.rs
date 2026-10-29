use std::sync::{Arc, Mutex};

use ironrdp_core::encode_vec;
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_w365::power_request::{PowerRequestClient, PowerRequestHandler, PowerRequestPdu};
use proptest::prelude::*;
use rstest::rstest;

use super::{CHANNEL_ID, encoded};

struct RecordingHandler(Arc<Mutex<Vec<bool>>>);

impl PowerRequestHandler for RecordingHandler {
    fn power_requests_changed(&mut self, active: bool) {
        self.0.lock().unwrap().push(active);
    }
}

fn started_client() -> (PowerRequestClient, Arc<Mutex<Vec<bool>>>) {
    let changes = Arc::new(Mutex::new(Vec::new()));
    let mut client = PowerRequestClient::new(Box::new(RecordingHandler(Arc::clone(&changes))));
    assert!(client.start(CHANNEL_ID).expect("start channel").is_empty());
    (client, changes)
}

fn info_changed(flags: u8) -> [u8; 10] {
    [
        0x03, 0x00, // POWER_REQUEST_INFO_CHANGED
        0x0A, 0x00, 0x00, 0x00, // pduLength
        flags, 0x00, 0x00, 0x00, // powerRequestFlags
    ]
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Power request redirection
#[test]
fn server_ready_is_answered_with_a_version_1_client_ready() {
    let (mut client, _) = started_client();
    let server_ready = [
        0x01, 0x00, // SERVER_READY
        0x0E, 0x00, 0x00, 0x00, // pduLength
        0x00, 0x00, 0x02, 0x00, // protocolVersion 2, answered with 1 regardless
        0x00, 0x00, 0x00, 0x00, // protocolFlags
    ];

    let replies = encoded(&client.process(CHANNEL_ID, &server_ready).expect("process PDU"));

    assert_eq!(
        replies,
        [vec![
            0x02, 0x00, // CLIENT_READY
            0x0E, 0x00, 0x00, 0x00, // pduLength
            0x00, 0x00, 0x01, 0x00, // protocolVersion 1
            0x00, 0x00, 0x00, 0x00, // protocolFlags
        ]]
    );
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Power request redirection
#[rstest]
#[case::current_requests(0x01, true)]
#[case::no_current_requests(0x02, false)]
fn power_request_state_reaches_the_handler(#[case] flags: u8, #[case] active: bool) {
    let (mut client, changes) = started_client();

    let replies = client.process(CHANNEL_ID, &info_changed(flags)).expect("process PDU");

    assert!(replies.is_empty());
    assert_eq!(*changes.lock().unwrap(), [active]);
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Power request redirection, which requires exactly one state flag
#[rstest]
#[case::both_flags(0x03)]
#[case::neither_flag(0x00)]
fn contradictory_power_request_flags_are_ignored(#[case] flags: u8) {
    let (mut client, changes) = started_client();

    client.process(CHANNEL_ID, &info_changed(flags)).expect("process PDU");

    assert!(changes.lock().unwrap().is_empty());
}

// oracle: mutant closing the channel keeps the power requests held (this client's choice)
#[test]
fn closing_while_active_releases_the_power_requests() {
    let (mut client, changes) = started_client();
    client.process(CHANNEL_ID, &info_changed(0x01)).expect("process PDU");

    client.close(CHANNEL_ID);

    assert_eq!(*changes.lock().unwrap(), [true, false]);
}

// oracle: mutant closing the channel reports a release the handler already received (this client's choice)
#[test]
fn closing_while_inactive_reports_nothing() {
    let (mut client, changes) = started_client();
    client.process(CHANNEL_ID, &info_changed(0x01)).expect("process PDU");
    client.process(CHANNEL_ID, &info_changed(0x02)).expect("process PDU");

    client.close(CHANNEL_ID);

    assert_eq!(*changes.lock().unwrap(), [true, false]);
}

fn any_pdu() -> impl Strategy<Value = PowerRequestPdu> {
    prop_oneof![
        (any::<u32>(), any::<u32>()).prop_map(|(version, flags)| PowerRequestPdu::ServerReady { version, flags }),
        (any::<u32>(), any::<u32>()).prop_map(|(version, flags)| PowerRequestPdu::ClientReady { version, flags }),
        any::<u32>().prop_map(|flags| PowerRequestPdu::PowerRequestInfoChanged { flags }),
    ]
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn pdu_round_trips_through_encode_and_decode(pdu in any_pdu()) {
        let bytes = encode_vec(&pdu).expect("encode PDU");

        prop_assert_eq!(ironrdp_core::decode::<PowerRequestPdu>(&bytes).expect("decode PDU"), pdu);
    }
}
