use std::sync::{Arc, Mutex};

use ironrdp_core::{DecodeErrorKind, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_w365::vdcswitch::client::{VdcSwitchClient, VdcSwitchHandler};
use ironrdp_w365::vdcswitch::pdu::{LaunchUri, LocalAction, ProtocolVersion, VdcSwitchPdu, VersionRequestPdu};
use proptest::prelude::*;
use rstest::rstest;

use super::{CHANNEL_ID, dvc_payloads, encoded};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    SwitchToLocal,
    LocalAction(LocalAction),
    SwitchToCloudCompleted(u32),
    NegotiationFailed(ProtocolVersion),
}

struct RecordingHandler {
    events: Arc<Mutex<Vec<Event>>>,
    switch_to_local_reply: Option<u32>,
}

impl VdcSwitchHandler for RecordingHandler {
    fn switch_to_local_requested(&mut self) -> Option<u32> {
        self.events.lock().unwrap().push(Event::SwitchToLocal);
        self.switch_to_local_reply
    }

    fn perform_local_action(&mut self, action: LocalAction) {
        self.events.lock().unwrap().push(Event::LocalAction(action));
    }

    fn switch_to_cloud_completed(&mut self, hresult: u32) {
        self.events.lock().unwrap().push(Event::SwitchToCloudCompleted(hresult));
    }

    fn version_negotiation_failed(&mut self, version: ProtocolVersion) {
        self.events.lock().unwrap().push(Event::NegotiationFailed(version));
    }
}

fn started_client(launch_uri: &str, switch_to_local_reply: Option<u32>) -> (VdcSwitchClient, Arc<Mutex<Vec<Event>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let handler = RecordingHandler {
        events: Arc::clone(&events),
        switch_to_local_reply,
    };
    let launch_uri = LaunchUri::new(launch_uri.to_owned()).expect("valid launch URI");
    let mut client = VdcSwitchClient::new(launch_uri, Box::new(handler));
    client.start(CHANNEL_ID).expect("start channel");
    (client, events)
}

fn process(client: &mut VdcSwitchClient, payload: &[u8]) -> Vec<Vec<u8>> {
    encoded(&client.process(CHANNEL_ID, payload).expect("process PDU"))
}

const VERSION_RESPONSE_V1: [u8; 10] = [
    0x02, 0x00, // VERSION_RESPONSE
    0x0A, 0x00, 0x00, 0x00, // pduLength
    0x00, 0x00, 0x01, 0x00, // version 1
];

const VERSION_RESPONSE_V2: [u8; 10] = [
    0x02, 0x00, // VERSION_RESPONSE
    0x0A, 0x00, 0x00, 0x00, // pduLength
    0x00, 0x00, 0x02, 0x00, // version 2
];

const SWITCH_TO_LOCAL_REQUEST: [u8; 6] = [
    0x03, 0x00, // SWITCH_TO_LOCAL_REQUEST
    0x06, 0x00, 0x00, 0x00, // pduLength
];

const PERFORM_BLUETOOTH_SETTINGS: [u8; 7] = [
    0x07, 0x00, // PERFORM_LOCAL_ACTION
    0x07, 0x00, 0x00, 0x00, // pduLength
    0x00, // Bluetooth settings
];

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Version request (type 1)
#[test]
fn channel_open_sends_version_request_with_null_terminated_launch_uri() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let handler = RecordingHandler {
        events,
        switch_to_local_reply: None,
    };
    let mut client = VdcSwitchClient::new(LaunchUri::new("ab".to_owned()).unwrap(), Box::new(handler));

    let messages = encoded(&client.start(CHANNEL_ID).expect("start channel"));

    assert_eq!(
        messages,
        [vec![
            0x01, 0x00, // VERSION_REQUEST
            0x18, 0x00, 0x00, 0x00, // pduLength: 2 * length + 20
            0x00, 0x00, 0x01, 0x00, // minimum version 1
            0x00, 0x00, 0x02, 0x00, // maximum version 2
            0x06, 0x00, 0x00, 0x00, // launch URI size: 2 * length + 2
            0x61, 0x00, 0x62, 0x00, 0x00, 0x00, // "ab" in UTF-16 with its terminator
        ]]
    );
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Switch-to-local request and response (types 3 and 4)
#[test]
fn switch_to_local_request_is_answered_with_the_handler_hresult() {
    let (mut client, events) = started_client("uri", Some(0x8000_4005));

    let replies = process(&mut client, &SWITCH_TO_LOCAL_REQUEST);

    assert_eq!(*events.lock().unwrap(), [Event::SwitchToLocal]);
    assert_eq!(
        replies,
        [vec![
            0x04, 0x00, // SWITCH_TO_LOCAL_RESPONSE
            0x0A, 0x00, 0x00, 0x00, // pduLength
            0x05, 0x40, 0x00, 0x80, // E_FAIL
        ]]
    );
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Switch-to-local request and response (types 3 and 4), which sends no response when the host application switches
#[test]
fn switch_to_local_request_handed_off_by_the_handler_sends_no_response() {
    let (mut client, events) = started_client("uri", None);

    let replies = process(&mut client, &SWITCH_TO_LOCAL_REQUEST);

    assert_eq!(*events.lock().unwrap(), [Event::SwitchToLocal]);
    assert!(replies.is_empty());
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Perform local action (type 7)
#[test]
fn local_action_reaches_the_handler_once_version_2_is_negotiated() {
    let (mut client, events) = started_client("uri", None);
    process(&mut client, &VERSION_RESPONSE_V2);

    let replies = process(&mut client, &PERFORM_BLUETOOTH_SETTINGS);

    assert_eq!(client.negotiated_version(), Some(ProtocolVersion::V2));
    assert_eq!(
        *events.lock().unwrap(),
        [Event::LocalAction(LocalAction::BLUETOOTH_SETTINGS)]
    );
    assert!(replies.is_empty());
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Perform local action (type 7), which requires version 2
#[rstest]
#[case::before_version_negotiation(None)]
#[case::under_version_1(Some(VERSION_RESPONSE_V1))]
fn local_action_is_ignored_without_version_2(#[case] version_response: Option<[u8; 10]>) {
    let (mut client, events) = started_client("uri", None);
    if let Some(version_response) = version_response {
        process(&mut client, &version_response);
    }

    process(&mut client, &PERFORM_BLUETOOTH_SETTINGS);

    assert!(events.lock().unwrap().is_empty());
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Version response (type 2), which closes the channel on an unsupported version
#[test]
fn unsupported_version_response_is_reported_and_silences_the_channel() {
    let (mut client, events) = started_client("uri", Some(0));
    let version_3 = [
        0x02, 0x00, // VERSION_RESPONSE
        0x0A, 0x00, 0x00, 0x00, // pduLength
        0x00, 0x00, 0x03, 0x00, // version 3
    ];
    process(&mut client, &version_3);

    let replies = process(&mut client, &SWITCH_TO_LOCAL_REQUEST);

    assert_eq!(client.negotiated_version(), None);
    assert_eq!(
        *events.lock().unwrap(),
        [Event::NegotiationFailed(ProtocolVersion(0x0003_0000))]
    );
    assert!(replies.is_empty());
    assert!(client.request_switch_to_cloud().is_err());
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Header, which checks only a minimum pduLength of 10
#[test]
fn version_response_with_trailing_fields_is_accepted() {
    let (mut client, _) = started_client("uri", None);
    let extended = [
        0x02, 0x00, // VERSION_RESPONSE
        0x0E, 0x00, 0x00, 0x00, // pduLength
        0x00, 0x00, 0x02, 0x00, // version 2
        0xAA, 0xBB, 0xCC, 0xDD, // field unknown to version 2
    ];

    process(&mut client, &extended);

    assert_eq!(client.negotiated_version(), Some(ProtocolVersion::V2));
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Header, which rejects a pduLength greater than the buffer
#[test]
fn pdu_length_past_the_payload_is_rejected() {
    let truncated = [
        0x02, 0x00, // VERSION_RESPONSE
        0x0E, 0x00, 0x00, 0x00, // pduLength claims 14 bytes
        0x00, 0x00, 0x02, 0x00, // only 10 present
    ];

    let error = ironrdp_core::decode::<VdcSwitchPdu>(&truncated).expect_err("truncated PDU");

    assert!(matches!(
        error.kind(),
        DecodeErrorKind::InvalidField { field: "pduLength", .. }
    ));
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Switch-to-cloud request and response (types 5 and 6)
#[test]
fn switch_to_cloud_request_is_a_bare_header() {
    let (client, _) = started_client("uri", None);

    let messages = client.request_switch_to_cloud().expect("open channel");

    assert_eq!(
        dvc_payloads(&messages),
        [vec![
            0x05, 0x00, // SWITCH_TO_CLOUD_REQUEST
            0x06, 0x00, 0x00, 0x00, // pduLength
        ]]
    );
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Switch-to-cloud request and response (types 5 and 6), which reads the HRESULT of the response
#[test]
fn switch_to_cloud_response_reports_its_hresult() {
    let (mut client, events) = started_client("uri", None);
    let response = [
        0x06, 0x00, // SWITCH_TO_CLOUD_RESPONSE
        0x0A, 0x00, 0x00, 0x00, // pduLength
        0x00, 0x00, 0x00, 0x00, // S_OK
    ];

    process(&mut client, &response);

    assert_eq!(*events.lock().unwrap(), [Event::SwitchToCloudCompleted(0)]);
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Virtual desktop control, Switch-to-cloud request and response (types 5 and 6), which fails with E_POINTER without a channel
#[test]
fn switch_to_cloud_request_needs_an_open_channel() {
    let (mut client, _) = started_client("uri", None);
    client.close(CHANNEL_ID);

    assert!(client.request_switch_to_cloud().is_err());
}

// oracle: mutant LaunchUri::new accepting a URI that cannot round-trip through its null-terminated encoding
#[test]
fn launch_uri_with_a_nul_is_rejected() {
    assert!(LaunchUri::new("a\0b".to_owned()).is_err());
}

// oracle: mutant VersionRequestPdu decoding a launch URI that breaks the no-NUL invariant
#[test]
fn version_request_with_an_embedded_nul_is_rejected() {
    let version_request = [
        0x01, 0x00, // VERSION_REQUEST
        0x1A, 0x00, 0x00, 0x00, // pduLength
        0x00, 0x00, 0x01, 0x00, // minimum version 1
        0x00, 0x00, 0x02, 0x00, // maximum version 2
        0x08, 0x00, 0x00, 0x00, // launch URI size
        0x61, 0x00, 0x00, 0x00, 0x62, 0x00, // "a", NUL, "b"
        0x00, 0x00, // terminator
    ];

    let error = ironrdp_core::decode::<VdcSwitchPdu>(&version_request).expect_err("embedded NUL");

    assert!(matches!(
        error.kind(),
        DecodeErrorKind::InvalidField { field: "launchUri", .. }
    ));
}

fn any_pdu() -> impl Strategy<Value = VdcSwitchPdu> {
    let launch_uri = "[^\u{0}]{0,64}".prop_map(|uri| LaunchUri::new(uri).expect("generated URI has no NUL"));
    prop_oneof![
        (any::<u32>(), any::<u32>(), launch_uri).prop_map(|(min, max, launch_uri)| {
            VdcSwitchPdu::VersionRequest(VersionRequestPdu {
                min_version: ProtocolVersion(min),
                max_version: ProtocolVersion(max),
                launch_uri,
            })
        }),
        any::<u32>().prop_map(|version| VdcSwitchPdu::VersionResponse {
            version: ProtocolVersion(version)
        }),
        Just(VdcSwitchPdu::SwitchToLocalRequest),
        any::<u32>().prop_map(|hresult| VdcSwitchPdu::SwitchToLocalResponse { hresult }),
        Just(VdcSwitchPdu::SwitchToCloudRequest),
        any::<u32>().prop_map(|hresult| VdcSwitchPdu::SwitchToCloudResponse { hresult }),
        any::<u8>().prop_map(|action| VdcSwitchPdu::PerformLocalAction {
            action: LocalAction(action)
        }),
    ]
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn pdu_round_trips_through_encode_and_decode(pdu in any_pdu()) {
        let bytes = encode_vec(&pdu).expect("encode PDU");

        prop_assert_eq!(ironrdp_core::decode::<VdcSwitchPdu>(&bytes).expect("decode PDU"), pdu);
    }
}
