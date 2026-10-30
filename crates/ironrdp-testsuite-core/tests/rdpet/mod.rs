use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
use ironrdp_rdpet::client::TelemetryClient;
use ironrdp_rdpet::pdu::TelemetryPdu;
use ironrdp_rdpet::server::{TelemetryHandler, TelemetryServer};
use rstest::rstest;

const CHANNEL_ID: u32 = 5;

/// A report with a distinct value in every field, so a swapped or misplaced field changes the bytes.
const REPORT: TelemetryPdu = TelemetryPdu {
    prompt_for_credentials_ms: 0x0403_0201,
    prompt_for_credentials_done_ms: 0x0807_0605,
    graphics_channel_opened_ms: 0x0C0B_0A09,
    first_graphics_received_ms: 0x100F_0E0D,
};

#[rustfmt::skip]
const REPORT_BYTES: [u8; 18] = [
    0x01, // Id
    0x12, // Length
    0x01, 0x02, 0x03, 0x04, // PromptForCredentialsMillis
    0x05, 0x06, 0x07, 0x08, // PromptForCredentialsDoneMillis
    0x09, 0x0A, 0x0B, 0x0C, // GraphicsChannelOpenedMillis
    0x0D, 0x0E, 0x0F, 0x10, // FirstGraphicsReceivedMillis
];

#[derive(Clone, Default)]
struct RecordingHandler(Arc<Mutex<Vec<TelemetryPdu>>>);

impl TelemetryHandler for RecordingHandler {
    fn report(&mut self, report: TelemetryPdu) {
        self.0.lock().unwrap().push(report);
    }
}

// oracle: spec MS-RDPET §2.2.1
#[test]
fn report_has_specified_wire_layout() {
    assert_eq!(encode_vec(&REPORT).unwrap(), REPORT_BYTES);
    assert_eq!(decode::<TelemetryPdu>(&REPORT_BYTES).unwrap(), REPORT);
}

// oracle: spec MS-RDPET §2.2.1
#[rstest]
#[case::id_other_than_one(&[0x02, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]
#[case::length_below_eighteen(&[0x01, 0x11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]
#[case::length_above_eighteen(&[0x01, 0x13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]
#[case::truncated(&[0x01, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]
fn decode_rejects_nonconforming_report(#[case] bytes: &[u8]) {
    assert!(decode::<TelemetryPdu>(bytes).is_err());
}

// oracle: spec MS-RDPET §2.1
#[test]
fn processors_use_specified_channel_name() {
    let server = TelemetryServer::new(Box::new(RecordingHandler::default()));

    assert_eq!(
        TelemetryClient::new().channel_name(),
        "Microsoft::Windows::RDS::Telemetry"
    );
    assert_eq!(server.channel_name(), "Microsoft::Windows::RDS::Telemetry");
}

// oracle: spec MS-RDPET §2.2.1
#[test]
fn client_sends_report_on_open_channel() {
    let mut client = TelemetryClient::new();
    assert!(client.start(CHANNEL_ID).unwrap().is_empty());

    let messages = client.send_report(REPORT).unwrap();

    assert_eq!(messages.len(), 1);
    let unframed = messages[0].encode_unframed_pdu().unwrap();
    let DrdynvcClientPdu::Data(data_pdu) = decode(&unframed).unwrap() else {
        panic!("expected DRDYNVC data");
    };
    assert_eq!(data_pdu.channel_id(), CHANNEL_ID);
    let DrdynvcDataPdu::Data(data) = data_pdu else {
        panic!("expected unsplit DRDYNVC data");
    };
    assert_eq!(data.data(), REPORT_BYTES);
}

// oracle: mutant sending a report before the channel opens or after it closes
#[test]
fn client_sends_no_report_without_open_channel() {
    let mut client = TelemetryClient::new();
    assert!(client.send_report(REPORT).is_err());

    client.start(CHANNEL_ID).unwrap();
    client.close(CHANNEL_ID);

    assert!(client.send_report(REPORT).is_err());
}

// oracle: spec MS-RDPET §3.1.5.1
#[test]
fn server_hands_report_to_handler() {
    let handler = RecordingHandler::default();
    let mut server = TelemetryServer::new(Box::new(handler.clone()));
    assert!(server.start(CHANNEL_ID).unwrap().is_empty());

    let responses = server.process(CHANNEL_ID, &REPORT_BYTES).unwrap();

    assert!(responses.is_empty());
    assert_eq!(*handler.0.lock().unwrap(), [REPORT]);
}

// oracle: spec MS-RDPET §3.1.5.1
#[rstest]
#[case::id_other_than_one(&[0x02, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]
#[case::length_other_than_eighteen(&[0x01, 0x11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]
#[case::truncated(&REPORT_BYTES[..17])]
fn server_ignores_malformed_report(#[case] bytes: &[u8]) {
    let handler = RecordingHandler::default();
    let mut server = TelemetryServer::new(Box::new(handler.clone()));

    let responses = server.process(CHANNEL_ID, bytes).unwrap();

    assert!(responses.is_empty());
    assert!(handler.0.lock().unwrap().is_empty());
}
