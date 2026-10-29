use std::sync::{Arc, Mutex};

use ironrdp_core::encode_vec;
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_w365::screen_capture::{
    ImageType, SaveResult, ScreenCaptureClient, ScreenCaptureHandler, ScreenCapturePdu,
};
use proptest::prelude::*;
use rstest::rstest;

use super::{CHANNEL_ID, dvc_payloads, encoded};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    UploadsReady,
    Saved(SaveResult),
}

struct RecordingHandler(Arc<Mutex<Vec<Event>>>);

impl ScreenCaptureHandler for RecordingHandler {
    fn uploads_ready(&mut self) {
        self.0.lock().unwrap().push(Event::UploadsReady);
    }

    fn save_completed(&mut self, result: SaveResult) {
        self.0.lock().unwrap().push(Event::Saved(result));
    }
}

const SERVER_READY_V1: [u8; 16] = [
    0x01, 0x00, // SERVER_READY
    0x00, 0x00, // flags
    0x08, 0x00, 0x00, 0x00, // payloadLength
    0x00, 0x00, 0x01, 0x00, // protocolVersion 1
    0x00, 0x00, 0x00, 0x00, // protocolFlags
];

fn started_client() -> (ScreenCaptureClient, Arc<Mutex<Vec<Event>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut client = ScreenCaptureClient::new(Box::new(RecordingHandler(Arc::clone(&events))));
    assert!(client.start(CHANNEL_ID).expect("start channel").is_empty());
    (client, events)
}

fn negotiated_client() -> (ScreenCaptureClient, Arc<Mutex<Vec<Event>>>) {
    let (mut client, events) = started_client();
    client
        .process(CHANNEL_ID, &SERVER_READY_V1)
        .expect("process server ready");
    events.lock().unwrap().clear();
    (client, events)
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §PDUs
#[test]
fn version_1_server_ready_is_answered_and_enables_uploads() {
    let (mut client, events) = started_client();

    let replies = encoded(&client.process(CHANNEL_ID, &SERVER_READY_V1).expect("process PDU"));

    assert_eq!(
        replies,
        [vec![
            0x02, 0x00, // CLIENT_READY
            0x00, 0x00, // flags
            0x08, 0x00, 0x00, 0x00, // payloadLength
            0x00, 0x00, 0x01, 0x00, // protocolVersion 1
            0x00, 0x00, 0x00, 0x00, // protocolFlags
        ]]
    );
    assert!(client.ready());
    assert_eq!(*events.lock().unwrap(), [Event::UploadsReady]);
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §PDUs, which accepts only protocol version 0x00010000
#[test]
fn other_server_versions_leave_uploads_disabled() {
    let (mut client, events) = started_client();
    let mut server_ready_v2 = SERVER_READY_V1;
    server_ready_v2[10] = 0x02;

    let replies = client.process(CHANNEL_ID, &server_ready_v2).expect("process PDU");

    assert!(replies.is_empty());
    assert!(!client.ready());
    assert!(events.lock().unwrap().is_empty());
    assert!(client.upload(ImageType::PNG, vec![0xAA]).is_err());
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Screen capture (type 3)
#[rstest]
#[case::png(ImageType::PNG, 0x01)]
#[case::jpeg(ImageType::JPEG, 0x02)]
fn upload_carries_the_image_type_capture_id_and_bytes(#[case] image_type: ImageType, #[case] flags: u8) {
    let (mut client, _) = negotiated_client();

    let (capture_id, messages) = client.upload(image_type, vec![0xAA, 0xBB, 0xCC]).expect("upload");

    assert_eq!(capture_id, 0);
    assert_eq!(
        dvc_payloads(&messages),
        [vec![
            0x03, 0x00, // SCREEN_CAPTURE
            flags, 0x00, // image type
            0x05, 0x00, 0x00, 0x00, // payloadLength: capture ID and image
            0x00, 0x00, // captureId
            0xAA, 0xBB, 0xCC, // image
        ]]
    );
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Screen capture (type 3), whose capture counter wraps after 0x7FFF
#[test]
fn capture_ids_count_up_and_wrap_after_0x7fff() {
    let (mut client, _) = negotiated_client();

    let ids: Vec<u16> = core::iter::repeat_with(|| client.upload(ImageType::PNG, vec![0]).expect("upload").0)
        .take(0x8001)
        .collect();

    assert_eq!(ids[..3], [0, 1, 2]);
    assert_eq!(ids[0x7FFF..], [0x7FFF, 0]);
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Screen capture (type 3), which rejects empty images
#[test]
fn empty_image_is_rejected() {
    let (mut client, _) = negotiated_client();

    assert!(client.upload(ImageType::PNG, Vec::new()).is_err());
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Save result (type 4)
#[test]
fn save_result_reaches_the_handler() {
    let (mut client, events) = negotiated_client();
    let save_result = [
        0x04, 0x00, // SAVE_RESULT
        0x00, 0x00, // flags
        0x18, 0x00, 0x00, 0x00, // payloadLength: 14 + 4 + 6
        0x05, 0x00, // captureId
        0x00, 0x00, 0x00, 0x00, // S_OK
        0x04, 0x00, 0x00, 0x00, // fileName size
        0x06, 0x00, 0x00, 0x00, // filePath size
        0x61, 0x00, 0x62, 0x00, // "ab"
        0x43, 0x00, 0x3A, 0x00, 0x5C, 0x00, // "C:\"
    ];

    let replies = client.process(CHANNEL_ID, &save_result).expect("process PDU");

    assert!(replies.is_empty());
    assert_eq!(
        *events.lock().unwrap(),
        [Event::Saved(SaveResult {
            capture_id: 5,
            hresult: 0,
            file_name: "ab".to_owned(),
            file_path: "C:\\".to_owned(),
        })]
    );
}

// oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Save result (type 4), which requires the string sizes to fill the payload
#[test]
fn save_result_with_inconsistent_string_sizes_is_ignored() {
    let (mut client, events) = negotiated_client();
    let save_result = [
        0x04, 0x00, // SAVE_RESULT
        0x00, 0x00, // flags
        0x12, 0x00, 0x00, 0x00, // payloadLength: 14 + 4
        0x05, 0x00, // captureId
        0x00, 0x00, 0x00, 0x00, // S_OK
        0x04, 0x00, 0x00, 0x00, // fileName size
        0x02, 0x00, 0x00, 0x00, // filePath size the payload does not hold
        0x61, 0x00, 0x62, 0x00, // "ab"
    ];

    client.process(CHANNEL_ID, &save_result).expect("process PDU");

    assert!(events.lock().unwrap().is_empty());
}

fn any_pdu() -> impl Strategy<Value = ScreenCapturePdu> {
    let text = "[^\u{0}]{0,32}";
    prop_oneof![
        (any::<u32>(), any::<u32>()).prop_map(|(version, flags)| ScreenCapturePdu::ServerReady { version, flags }),
        (any::<u32>(), any::<u32>()).prop_map(|(version, flags)| ScreenCapturePdu::ClientReady { version, flags }),
        (
            any::<u16>(),
            any::<u16>(),
            proptest::collection::vec(any::<u8>(), 0..64)
        )
            .prop_map(|(image_type, capture_id, image)| ScreenCapturePdu::ScreenCapture {
                image_type: ImageType(image_type),
                capture_id,
                image,
            }),
        (0..=0x7FFFu16, any::<u32>(), text, text).prop_map(|(capture_id, hresult, file_name, file_path)| {
            ScreenCapturePdu::SaveResult(SaveResult {
                capture_id,
                hresult,
                file_name,
                file_path,
            })
        }),
    ]
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn pdu_round_trips_through_encode_and_decode(pdu in any_pdu()) {
        let bytes = encode_vec(&pdu).expect("encode PDU");

        prop_assert_eq!(ironrdp_core::decode::<ScreenCapturePdu>(&bytes).expect("decode PDU"), pdu);
    }
}
