use core::error::Error as _;
use std::sync::{Arc, Mutex};

use ironrdp_bulk as _;
use ironrdp_core::{DecodeError, DecodeErrorKind, encode_vec};
use ironrdp_dvc::pdu::{DataFirstPdu, DataPdu, DrdynvcDataPdu};
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{PduError, PduErrorKind, PduResult};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};
use tracing as _;

type Messages = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;

struct RecordingDvc {
    limit: Option<usize>,
    messages: Messages,
}

ironrdp_core::impl_as_any!(RecordingDvc);

impl DvcProcessor for RecordingDvc {
    fn channel_name(&self) -> &str {
        "test::transport-limit"
    }

    fn max_message_size(&self) -> Option<usize> {
        self.limit
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        assert_eq!(channel_id, 7);
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.messages
            .lock()
            .expect("message lock")
            .push((channel_id, payload.to_vec()));
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for RecordingDvc {}

fn client(carrier_limit: Option<usize>, dvc_limit: Option<usize>) -> (DrdynvcClient, Messages) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let mut client = DrdynvcClient::new();
    if let Some(limit) = carrier_limit {
        client = client.with_max_transport_message_size(limit);
    }
    client
        .attach_established_dynamic_channel(
            7,
            RecordingDvc {
                limit: dvc_limit,
                messages: Arc::clone(&messages),
            },
        )
        .expect("established DVC");
    (client, messages)
}

fn send(client: &mut DrdynvcClient, pdu: DrdynvcDataPdu) -> PduResult<Vec<SvcMessage>> {
    client.process(&encode_vec(&pdu).expect("encode carrier"))
}

fn assert_carrier_error(error: &PduError) {
    assert!(matches!(
        error.kind(),
        PduErrorKind::Other {
            description: "DRDYNVC carrier message exceeds configured maximum size"
        }
    ));
    assert_eq!(
        error.kind().to_string(),
        "other (DRDYNVC carrier message exceeds configured maximum size)"
    );
}

#[test]
fn transport_limit_getter_preserves_default_and_builder_values() {
    assert_eq!(DrdynvcClient::new().max_message_size(), None);
    assert_eq!(DrdynvcClient::default().max_message_size(), None);
    for limit in [0, 1, 6, usize::MAX] {
        let client = DrdynvcClient::new().with_max_transport_message_size(limit);
        assert_eq!(client.max_message_size(), Some(limit));
    }
    let (client, messages) = client(Some(0), Some(8));
    let mut client = client.with_max_transport_message_size(6);
    assert_eq!(client.max_message_size(), Some(6));
    assert!(
        send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![1, 2, 3, 4])))
            .expect("builder retains established channel")
            .is_empty()
    );
    assert_eq!(*messages.lock().expect("message lock"), [(7, vec![1, 2, 3, 4])]);
}

#[test]
fn direct_carrier_limit_checks_before_decode_and_recovers() {
    let (mut client, messages) = client(Some(6), Some(8));
    assert_carrier_error(&client.process(&[0; 7]).expect_err("size checked before invalid PDU"));
    assert_carrier_error(&client.process(&[0; 16]).expect_err("later blocked size"));
    assert!(messages.lock().expect("message lock").is_empty());
    for data in [vec![], vec![1], vec![1, 2, 3], vec![1, 2, 3, 4]] {
        assert!(
            send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, data)))
                .expect("below or at carrier cap")
                .is_empty()
        );
    }
    assert_eq!(
        *messages.lock().expect("message lock"),
        [(7, vec![]), (7, vec![1]), (7, vec![1, 2, 3]), (7, vec![1, 2, 3, 4])]
    );
    let error = send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![0xaa; 5]))).expect_err("cap plus one");
    assert_carrier_error(&error);
    assert_eq!(messages.lock().expect("message lock").len(), 4);
    assert!(
        send(&mut client, DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 3, vec![9])))
            .expect("fresh first")
            .is_empty()
    );
    assert_eq!(messages.lock().expect("message lock").len(), 4);
    assert!(
        send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![8, 7])))
            .expect("fresh last")
            .is_empty()
    );
    assert_eq!(messages.lock().expect("message lock").last(), Some(&(7, vec![9, 8, 7])));
}

#[test]
fn dynamic_message_can_span_multiple_bounded_carriers() {
    let (mut client, messages) = client(Some(6), Some(8));
    for pdu in [
        DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 8, vec![1, 2, 3])),
        DrdynvcDataPdu::Data(DataPdu::new(7, vec![4, 5, 6, 7])),
    ] {
        assert_eq!(encode_vec(&pdu).expect("encode carrier").len(), 6);
        assert!(send(&mut client, pdu).expect("bounded carrier").is_empty());
        assert!(messages.lock().expect("message lock").is_empty());
    }
    assert!(
        send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![8])))
            .expect("complete dynamic message")
            .is_empty()
    );
    assert_eq!(
        *messages.lock().expect("message lock"),
        [(7, vec![1, 2, 3, 4, 5, 6, 7, 8])]
    );
}

#[test]
fn carrier_rejection_preserves_pending_dynamic_message() {
    let (mut client, messages) = client(Some(6), Some(8));
    assert!(
        send(
            &mut client,
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 8, vec![1, 2, 3]))
        )
        .expect("first")
        .is_empty()
    );
    assert_carrier_error(
        &send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![0xaa; 5]))).expect_err("oversize carrier"),
    );
    assert!(messages.lock().expect("message lock").is_empty());
    assert!(
        send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![4, 5, 6, 7])))
            .expect("valid continuation")
            .is_empty()
    );
    assert!(messages.lock().expect("message lock").is_empty());
    assert!(
        send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![8])))
            .expect("final continuation")
            .is_empty()
    );
    assert_eq!(
        *messages.lock().expect("message lock"),
        [(7, vec![1, 2, 3, 4, 5, 6, 7, 8])]
    );
}

#[test]
fn per_dvc_limit_rejects_even_when_carrier_fits() {
    for pdu in [
        DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 9, vec![1])),
        DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 16, vec![1])),
        DrdynvcDataPdu::Data(DataPdu::new(7, vec![1; 9])),
        DrdynvcDataPdu::Data(DataPdu::new(7, vec![1; 12])),
    ] {
        let (mut client, messages) = client(Some(16), Some(8));
        assert!(encode_vec(&pdu).expect("encode carrier").len() <= 16);
        let field = match &pdu {
            DrdynvcDataPdu::DataFirst(_) => "DataFirstPdu::length",
            DrdynvcDataPdu::Data(_) => "DataPdu::data",
            _ => panic!("test input must be DataFirst or Data"),
        };
        let error = send(&mut client, pdu).expect_err("per-DVC cap");
        assert!(matches!(error.kind(), PduErrorKind::Decode));
        let source = error
            .source()
            .expect("decode source")
            .downcast_ref::<DecodeError>()
            .expect("structured decode error");
        assert!(matches!(
            source.kind(),
            DecodeErrorKind::InvalidField { field: actual, reason: "DVC message exceeds configured maximum size", offset: None }
                if *actual == field
        ));
        assert!(messages.lock().expect("message lock").is_empty());
        assert!(
            send(&mut client, DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 8, vec![9])))
                .expect("fresh first")
                .is_empty()
        );
        assert!(messages.lock().expect("message lock").is_empty());
        assert!(
            send(
                &mut client,
                DrdynvcDataPdu::Data(DataPdu::new(7, vec![8, 7, 6, 5, 4, 3, 2]))
            )
            .expect("exact DVC cap")
            .is_empty()
        );
        assert_eq!(
            *messages.lock().expect("message lock"),
            [(7, vec![9, 8, 7, 6, 5, 4, 3, 2])]
        );
    }
}

#[test]
fn zero_carrier_limit_rejects_empty_decode_and_nonempty_size_separately() {
    let mut client = DrdynvcClient::new().with_max_transport_message_size(0);
    let error = client.process(&[]).expect_err("empty PDU");
    assert!(matches!(error.kind(), PduErrorKind::Decode));
    let source = error
        .source()
        .expect("decode source")
        .downcast_ref::<DecodeError>()
        .expect("structured decode error");
    assert!(matches!(
        source.kind(),
        DecodeErrorKind::NotEnoughBytes {
            received: 0,
            expected: 1,
            offset: Some(0)
        }
    ));
    assert_carrier_error(&client.process(&[0]).expect_err("one byte exceeds zero"));
}

#[test]
fn unlimited_carrier_accepts_data_larger_than_a_bounded_carrier() {
    for carrier_limit in [None, Some(usize::MAX)] {
        let (mut client, messages) = client(carrier_limit, None);
        assert!(
            send(&mut client, DrdynvcDataPdu::Data(DataPdu::new(7, vec![0x5a; 4097])))
                .expect("unlimited carrier")
                .is_empty()
        );
        assert_eq!(*messages.lock().expect("message lock"), [(7, vec![0x5a; 4097])]);
    }
}
