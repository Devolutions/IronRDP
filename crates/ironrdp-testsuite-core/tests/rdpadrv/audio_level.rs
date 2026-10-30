use std::sync::{Arc, Mutex};

use ironrdp_core::encode_vec;
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpadrv::client::{AudioLevelClient, AudioLevelHandler};
use ironrdp_rdpadrv::pdu::{AudioLevelPdu, DataFlow, VolumeChange};
use ironrdp_rdpadrv::server::{AudioLevelServer, AudioLevelServerHandler};
use proptest::prelude::*;
use rstest::rstest;

use super::{CHANNEL_ID, encoded};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<VolumeChange>>>);

impl Recorder {
    fn take(&self) -> Vec<VolumeChange> {
        core::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl AudioLevelHandler for Recorder {
    fn volume_changed(&mut self, change: VolumeChange) {
        self.0.lock().unwrap().push(change);
    }
}

impl AudioLevelServerHandler for Recorder {
    fn volume_requested(&mut self, change: VolumeChange) {
        self.0.lock().unwrap().push(change);
    }
}

const SAE_STARTED: [u8; 4] = [0x01, 0x00, 0x00, 0x00];
const SAE_REMOTE_CONNECT: [u8; 4] = [0x03, 0x00, 0x00, 0x00];

const RENDER_HALF: VolumeChange = VolumeChange {
    data_flow: DataFlow::Render,
    volume: 0.5,
    muted: false,
};

const CAPTURE_MUTED: VolumeChange = VolumeChange {
    data_flow: DataFlow::Capture,
    volume: 0.25,
    muted: true,
};

fn started_client(client: AudioLevelClient) -> AudioLevelClient {
    let mut client = client;
    assert!(client.start(CHANNEL_ID).expect("start channel").is_empty());
    client
}

// oracle: spec MS-RDPADRV §2.2.2
#[test]
fn volume_change_has_specified_wire_layout() {
    let bytes = [
        0x02, 0x00, 0x00, 0x00, // eEvent: SAE_VolumeChange
        0x01, 0x00, 0x00, 0x00, // eDataFlow: eCapture
        0x00, 0x00, 0x80, 0x3E, // lVolume: 0.25 as an IEEE 754 single
        0x01, 0x00, 0x00, 0x00, // fMuted: TRUE
    ];

    assert_eq!(
        encode_vec(&AudioLevelPdu::VolumeChange(CAPTURE_MUTED)).expect("encode"),
        bytes
    );
    assert_eq!(
        ironrdp_core::decode::<AudioLevelPdu>(&bytes).expect("decode"),
        AudioLevelPdu::VolumeChange(CAPTURE_MUTED)
    );
}

// oracle: spec MS-RDPADRV §2.2.2
#[test]
fn volume_change_with_unknown_data_flow_is_rejected() {
    let bytes = [
        0x02, 0x00, 0x00, 0x00, // eEvent: SAE_VolumeChange
        0x02, 0x00, 0x00, 0x00, // eDataFlow: neither eRender nor eCapture
        0x00, 0x00, 0x00, 0x3F, // lVolume: 0.5
        0x00, 0x00, 0x00, 0x00, // fMuted: FALSE
    ];

    assert!(ironrdp_core::decode::<AudioLevelPdu>(&bytes).is_err());
}

// oracle: spec MS-RDPADRV §3.1.3.1, §4.1
#[rstest]
#[case::new_session(SAE_STARTED)]
#[case::reconnected_session(SAE_REMOTE_CONNECT)]
fn initialization_is_answered_with_each_stored_volume(#[case] initialization: [u8; 4]) {
    let mut client = started_client(
        AudioLevelClient::new(Box::new(Recorder::default()))
            .with_volume(CAPTURE_MUTED)
            .with_volume(RENDER_HALF),
    );

    let replies = encoded(&client.process(CHANNEL_ID, &initialization).expect("process"));

    assert_eq!(
        replies,
        [
            vec![
                0x02, 0x00, 0x00, 0x00, // eEvent: SAE_VolumeChange
                0x00, 0x00, 0x00, 0x00, // eDataFlow: eRender
                0x00, 0x00, 0x00, 0x3F, // lVolume: 0.5
                0x00, 0x00, 0x00, 0x00, // fMuted: FALSE
            ],
            vec![
                0x02, 0x00, 0x00, 0x00, // eEvent: SAE_VolumeChange
                0x01, 0x00, 0x00, 0x00, // eDataFlow: eCapture
                0x00, 0x00, 0x80, 0x3E, // lVolume: 0.25
                0x01, 0x00, 0x00, 0x00, // fMuted: TRUE
            ],
        ]
    );
}

// oracle: spec MS-RDPADRV §3.1.3.1
#[test]
fn initialization_without_stored_volumes_is_not_answered() {
    let mut client = started_client(AudioLevelClient::new(Box::new(Recorder::default())));

    assert!(client.process(CHANNEL_ID, &SAE_STARTED).expect("process").is_empty());
}

// oracle: spec MS-RDPADRV §3.1.5
#[test]
fn reported_volume_replaces_the_stored_one_without_an_answer() {
    let recorder = Recorder::default();
    let mut client = started_client(AudioLevelClient::new(Box::new(recorder.clone())).with_volume(RENDER_HALF));
    let louder = VolumeChange {
        volume: 0.75,
        ..RENDER_HALF
    };

    let replies = client
        .process(
            CHANNEL_ID,
            &encode_vec(&AudioLevelPdu::VolumeChange(louder)).expect("encode"),
        )
        .expect("process");

    assert!(replies.is_empty());
    assert_eq!(recorder.take(), [louder]);
    let replies = encoded(&client.process(CHANNEL_ID, &SAE_REMOTE_CONNECT).expect("process"));
    assert_eq!(
        replies,
        [encode_vec(&AudioLevelPdu::VolumeChange(louder)).expect("encode")]
    );
}

// oracle: spec MS-RDPADRV §3.1.3.1, §4.1
#[rstest]
#[case::new_session(|server| server, SAE_STARTED)]
#[case::reconnected_session(AudioLevelServer::reconnected, SAE_REMOTE_CONNECT)]
fn server_initializes_and_receives_the_client_volumes(
    #[case] configure: fn(AudioLevelServer) -> AudioLevelServer,
    #[case] initialization: [u8; 4],
) {
    let requested = Recorder::default();
    let mut server = configure(AudioLevelServer::new(Box::new(requested.clone())));
    let mut client = started_client(
        AudioLevelClient::new(Box::new(Recorder::default()))
            .with_volume(RENDER_HALF)
            .with_volume(CAPTURE_MUTED),
    );

    let opening = encoded(&server.start(CHANNEL_ID).expect("start server"));
    assert_eq!(opening, [initialization.to_vec()]);
    for reply in encoded(&client.process(CHANNEL_ID, &opening[0]).expect("process")) {
        assert!(server.process(CHANNEL_ID, &reply).expect("process").is_empty());
    }

    assert_eq!(requested.take(), [RENDER_HALF, CAPTURE_MUTED]);
}

fn any_volume_change() -> impl Strategy<Value = VolumeChange> {
    (
        prop_oneof![Just(DataFlow::Render), Just(DataFlow::Capture)],
        0.0f32..=1.0,
        any::<bool>(),
    )
        .prop_map(|(data_flow, volume, muted)| VolumeChange {
            data_flow,
            volume,
            muted,
        })
}

fn any_pdu() -> impl Strategy<Value = AudioLevelPdu> {
    prop_oneof![
        Just(AudioLevelPdu::Started),
        Just(AudioLevelPdu::RemoteConnect),
        any_volume_change().prop_map(AudioLevelPdu::VolumeChange),
    ]
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn pdu_round_trips_through_encode_and_decode(pdu in any_pdu()) {
        let bytes = encode_vec(&pdu).expect("encode PDU");

        prop_assert_eq!(ironrdp_core::decode::<AudioLevelPdu>(&bytes).expect("decode PDU"), pdu);
    }
}
