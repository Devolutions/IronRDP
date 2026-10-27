use std::sync::{Arc, Mutex};

use ironrdp_bulk as _;
use ironrdp_core::encode_vec;
use ironrdp_dvc::pdu::{CreateRequestPdu, DataFirstPdu, DataPdu, DrdynvcDataPdu, DrdynvcServerPdu};
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcProcessor as _;
use tracing as _;

const CHANNEL_ID: u32 = 7;
const CHANNEL_NAME: &str = "test::bounded-reassembly";
const MAX_MESSAGE_SIZE: usize = 4;

struct RecordingProcessor {
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl DvcProcessor for RecordingProcessor {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_MESSAGE_SIZE)
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.messages.lock().expect("test message lock").push(payload.to_vec());
        Ok(Vec::new())
    }
}

ironrdp_core::impl_as_any!(RecordingProcessor);

impl DvcClientProcessor for RecordingProcessor {}

fn client() -> (DrdynvcClient, Arc<Mutex<Vec<Vec<u8>>>>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let processor = RecordingProcessor {
        messages: Arc::clone(&messages),
    };
    let mut client = DrdynvcClient::new().with_dynamic_channel(processor);
    let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(CHANNEL_ID, CHANNEL_NAME.to_owned()));
    client
        .process(&encode_vec(&create).expect("encode create request"))
        .expect("create dynamic channel");
    (client, messages)
}

fn send(client: &mut DrdynvcClient, pdu: DrdynvcDataPdu) -> PduResult<usize> {
    client
        .process(&encode_vec(&pdu).expect("encode DVC data"))
        .map(|responses| responses.len())
}

#[test]
fn fragmented_message_at_exact_cap_is_accepted() {
    let (mut client, messages) = client();

    assert_eq!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(CHANNEL_ID, 4, vec![9, 10, 11, 12])),
        )
        .expect("complete DataFirst"),
        0
    );
    assert_eq!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(CHANNEL_ID, 4, vec![1, 2])),
        )
        .expect("first fragment"),
        0
    );
    assert_eq!(
        send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![3, 4])),).expect("final fragment"),
        0
    );

    assert_eq!(
        *messages.lock().expect("test message lock"),
        [vec![9, 10, 11, 12], vec![1, 2, 3, 4]]
    );
}

#[test]
fn advertised_total_over_cap_is_rejected_without_retaining_first_fragment() {
    let (mut client, messages) = client();

    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(CHANNEL_ID, 5, vec![0xaa, 0xbb])),
        )
        .is_err()
    );
    assert!(send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![0xcc])),).is_ok());

    assert_eq!(*messages.lock().expect("test message lock"), [vec![0xcc]]);
}

#[test]
fn first_fragment_bytes_over_cap_are_rejected_without_dispatch() {
    let (mut client, messages) = client();

    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(CHANNEL_ID, 4, vec![1, 2, 3, 4, 5])),
        )
        .is_err()
    );

    assert!(messages.lock().expect("test message lock").is_empty());
}

#[test]
fn later_fragment_overrun_is_rejected_without_dispatch() {
    let (mut client, messages) = client();

    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(CHANNEL_ID, 4, vec![1, 2])),
        )
        .is_ok()
    );
    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![3, 4, 5])),
        )
        .is_err()
    );
    assert!(send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![9])),).is_ok());

    assert_eq!(*messages.lock().expect("test message lock"), [vec![9]]);
}

#[test]
fn valid_new_message_proceeds_after_fragment_overrun() {
    let (mut client, messages) = client();

    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(CHANNEL_ID, 4, vec![1, 2])),
        )
        .is_ok()
    );
    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![3, 4, 5])),
        )
        .is_err()
    );
    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(CHANNEL_ID, 3, vec![6])),
        )
        .is_ok()
    );
    assert!(send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![7, 8])),).is_ok());

    assert_eq!(*messages.lock().expect("test message lock"), [vec![6, 7, 8]]);
}

#[test]
fn unfragmented_messages_are_rejected_only_when_over_cap() {
    let (mut client, messages) = client();

    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![1, 2, 3, 4])),
        )
        .is_ok()
    );
    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, vec![1, 2, 3, 4, 5])),
        )
        .is_err()
    );

    assert_eq!(*messages.lock().expect("test message lock"), [vec![1, 2, 3, 4]]);
}
