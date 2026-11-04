use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ironrdp_core::encode_vec;
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_w365::battery::{BatteryInfo, BatteryPdu, BatteryState, BatteryStatusClient, BatteryStatusHandler};
use proptest::prelude::*;

use super::{CHANNEL_ID, dvc_payloads, encoded};

const AC_80_PERCENT: BatteryInfo = BatteryInfo {
    state: BatteryState::AC_POWER,
    percent: 80,
    minutes_remaining: 0,
};

const SERVER_READY: [u8; 14] = [
    0x01, 0x00, // SERVER_READY
    0x0E, 0x00, 0x00, 0x00, // pduLength
    0x00, 0x00, 0x01, 0x00, // protocolVersion
    0x00, 0x00, 0x00, 0x00, // protocolFlags
];

struct CountingHandler(Arc<AtomicUsize>);

impl BatteryStatusHandler for CountingHandler {
    fn battery_status_requested(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn started_client_counting_requests() -> (BatteryStatusClient, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let mut client = BatteryStatusClient::new(Box::new(CountingHandler(Arc::clone(&requests))));
    assert!(client.start(CHANNEL_ID).expect("start channel").is_empty());
    (client, requests)
}

fn started_client() -> BatteryStatusClient {
    started_client_counting_requests().0
}

fn info_request(state: u8, percent: u8) -> [u8; 16] {
    [
        0x03, 0x00, // BATTERY_INFO_REQUEST
        0x10, 0x00, 0x00, 0x00, // pduLength
        state, 0x00, 0x00, 0x00, // batteryState the Cloud PC shows
        percent, 0x00, // batteryLifePercent the Cloud PC shows
        0x00, 0x00, 0x00, 0x00, // batteryLifeMinutes
    ]
}

fn request(client: &mut BatteryStatusClient, state: u8, percent: u8) -> Vec<Vec<u8>> {
    encoded(
        &client
            .process(CHANNEL_ID, &info_request(state, percent))
            .expect("process PDU"),
    )
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §PDUs
#[test]
fn server_ready_is_answered_with_a_version_1_client_ready() {
    let mut client = started_client();

    let replies = encoded(&client.process(CHANNEL_ID, &SERVER_READY).expect("process PDU"));

    assert_eq!(
        replies,
        [vec![
            0x02, 0x00, // CLIENT_READY
            0x10, 0x00, 0x00, 0x00, // pduLength
            0x00, 0x00, 0x01, 0x00, // protocolVersion 1
            0x00, 0x00, 0x00, 0x00, // protocolFlags
            0x00, 0x00, // reserved
        ]]
    );
}

// oracle: mutant no request for a reading when the server becomes ready (this client's choice)
#[test]
fn server_ready_asks_the_handler_for_a_reading() {
    let (mut client, requests) = started_client_counting_requests();
    assert_eq!(requests.load(Ordering::Relaxed), 0);

    client.process(CHANNEL_ID, &SERVER_READY).expect("process PDU");

    assert_eq!(requests.load(Ordering::Relaxed), 1);
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Pacing, which sends an update only once the server requests one
#[test]
fn reading_is_sent_when_the_server_requests_it() {
    let mut client = started_client();
    let reading = BatteryInfo {
        state: BatteryState::ON_BATTERY,
        percent: 0x41,
        minutes_remaining: 0x12C,
    };
    assert!(client.update_battery(reading).expect("record reading").is_empty());

    let replies = request(&mut client, 0x02, 0x64);

    assert_eq!(
        replies,
        [vec![
            0x04, 0x00, // BATTERY_INFO_CHANGED
            0x10, 0x00, 0x00, 0x00, // pduLength
            0x00, 0x00, 0x00, 0x00, // on battery
            0x41, 0x00, // 65 percent
            0x2C, 0x01, 0x00, 0x00, // 300 minutes remaining
        ]]
    );
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Pacing, which uses up the permission with each update
#[test]
fn each_server_request_allows_one_update() {
    let mut client = started_client();
    client.update_battery(AC_80_PERCENT).expect("record reading");
    assert_eq!(request(&mut client, 0x01, 0x00).len(), 1);

    let unsolicited = client
        .update_battery(BatteryInfo {
            percent: 81,
            ..AC_80_PERCENT
        })
        .expect("record reading");
    let next = request(&mut client, 0x02, 80);

    assert!(unsolicited.is_empty());
    assert_eq!(next.len(), 1);
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Pacing, which sends only when the state or percentage differs from the server's
#[test]
fn reading_the_server_already_shows_is_held_until_it_changes() {
    let mut client = started_client();
    client.update_battery(AC_80_PERCENT).expect("record reading");
    request(&mut client, 0x01, 0x00);

    let same = request(&mut client, 0x02, 80);
    let minutes_only = client
        .update_battery(BatteryInfo {
            minutes_remaining: 42,
            ..AC_80_PERCENT
        })
        .expect("record reading");
    let changed = client
        .update_battery(BatteryInfo {
            state: BatteryState::ON_BATTERY,
            percent: 80,
            minutes_remaining: 42,
        })
        .expect("record reading");

    assert!(same.is_empty());
    assert!(minutes_only.is_empty());
    assert_eq!(
        dvc_payloads(&changed),
        [vec![
            0x04, 0x00, // BATTERY_INFO_CHANGED
            0x10, 0x00, 0x00, 0x00, // pduLength
            0x00, 0x00, 0x00, 0x00, // on battery
            0x50, 0x00, // 80 percent
            0x2A, 0x00, 0x00, 0x00, // 42 minutes remaining
        ]]
    );
}

fn any_info() -> impl Strategy<Value = BatteryInfo> {
    (any::<u32>(), any::<u16>(), any::<u32>()).prop_map(|(state, percent, minutes_remaining)| BatteryInfo {
        state: BatteryState(state),
        percent,
        minutes_remaining,
    })
}

fn any_pdu() -> impl Strategy<Value = BatteryPdu> {
    prop_oneof![
        (any::<u32>(), any::<u32>()).prop_map(|(version, flags)| BatteryPdu::ServerReady { version, flags }),
        (any::<u32>(), any::<u32>()).prop_map(|(version, flags)| BatteryPdu::ClientReady { version, flags }),
        any_info().prop_map(BatteryPdu::BatteryInfoRequest),
        any_info().prop_map(BatteryPdu::BatteryInfoChanged),
    ]
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn pdu_round_trips_through_encode_and_decode(pdu in any_pdu()) {
        let bytes = encode_vec(&pdu).expect("encode PDU");

        prop_assert_eq!(ironrdp_core::decode::<BatteryPdu>(&bytes).expect("decode PDU"), pdu);
    }
}
