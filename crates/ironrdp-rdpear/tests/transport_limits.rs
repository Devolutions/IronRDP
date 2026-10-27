use core::error::Error as _;
use std::sync::{Arc, Mutex};

use ironrdp_core::{Decode as _, DecodeError, DecodeErrorKind, ReadCursor, encode_vec};
use ironrdp_dvc::DrdynvcClient;
use ironrdp_dvc::pdu::{ClosePdu, DataFirstPdu, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu, DrdynvcServerPdu};
use ironrdp_pdu::{PduError, PduErrorKind, PduResult};
use ironrdp_rdpear::{AuthRedirectionBackend, AuthRedirectionClient, AuthRedirectionPdu};
use ironrdp_svc::{StaticVirtualChannel, SvcMessage};

const FRAME: &[u8] = &[
    0xc8, 0xc3, 0xac, 0x4e, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xde, 0xad, 0xbe, 0xef,
];
const FIRST: u32 = 1;
const LAST: u32 = 2;
const COMPRESSED: u32 = 0x0020_0000;

#[derive(Default)]
struct Calls {
    requests: Vec<(Vec<u8>, usize)>,
    closes: usize,
}

struct Backend(Arc<Mutex<Calls>>);

impl AuthRedirectionBackend for Backend {
    fn process(&mut self, encrypted_payload: &[u8], max_payload_size: usize) -> PduResult<Vec<u8>> {
        self.0
            .lock()
            .expect("backend lock")
            .requests
            .push((encrypted_payload.to_vec(), max_payload_size));
        Ok(vec![0x42])
    }

    fn close(&mut self) {
        self.0.lock().expect("backend lock").closes += 1;
    }
}

fn channel(carrier_limit: Option<usize>) -> (StaticVirtualChannel, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let auth = AuthRedirectionClient::new(Box::new(Backend(Arc::clone(&calls))), 28).expect("RDPEAR limit");
    let mut client = DrdynvcClient::new();
    if let Some(limit) = carrier_limit {
        client = client.with_max_transport_message_size(limit);
    }
    client
        .attach_established_dynamic_channel(7, auth)
        .expect("established RDPEAR");
    (StaticVirtualChannel::new(client), calls)
}

fn svc_frame(length: u32, flags: u32, data: &[u8]) -> Vec<u8> {
    let mut frame = length.to_le_bytes().to_vec();
    frame.extend_from_slice(&flags.to_le_bytes());
    frame.extend_from_slice(data);
    frame
}

fn assert_response(responses: Vec<SvcMessage>) {
    assert_eq!(responses.len(), 1);
    let bytes = responses[0].encode_unframed_pdu().expect("response bytes");
    let mut cursor = ReadCursor::new(&bytes);
    let pdu = DrdynvcClientPdu::decode(&mut cursor).expect("DVC response");
    assert!(cursor.is_empty());
    let data = match pdu {
        DrdynvcClientPdu::Data(data) => Some(data),
        _ => None,
    }
    .expect("expected DVC data response");
    assert_eq!(data.channel_id(), 7);
    let data = match data {
        DrdynvcDataPdu::Data(data) => Some(data),
        _ => None,
    }
    .expect("expected single DVC Data response");
    let mut expected = FRAME[..24].to_vec();
    expected[4..8].copy_from_slice(&1u32.to_le_bytes());
    expected.push(0x42);
    assert_eq!(data.data(), expected);
    let reply = AuthRedirectionPdu::decode(&mut ReadCursor::new(data.data())).expect("RDPEAR response");
    assert_eq!(reply.reserved, 0);
    assert_eq!(reply.encrypted_payload, [0x42]);
}

fn dispatch_valid_fragments(channel: &mut StaticVirtualChannel, calls: &Arc<Mutex<Calls>>) {
    let first = encode_vec(&DrdynvcDataPdu::DataFirst(DataFirstPdu::new(
        7,
        28,
        FRAME[..10].to_vec(),
    )))
    .expect("first carrier");
    let last = encode_vec(&DrdynvcDataPdu::Data(DataPdu::new(7, FRAME[10..].to_vec()))).expect("last carrier");
    assert_eq!(first.len(), 13);
    assert_eq!(last.len(), 20);
    assert!(
        channel
            .process(&svc_frame(13, FIRST, &first[..6]))
            .expect("SVC first")
            .is_empty()
    );
    assert!(calls.lock().expect("backend lock").requests.is_empty());
    assert!(
        channel
            .process(&svc_frame(13, LAST, &first[6..]))
            .expect("DVC first")
            .is_empty()
    );
    assert!(calls.lock().expect("backend lock").requests.is_empty());
    assert!(
        channel
            .process(&svc_frame(20, FIRST, &last[..9]))
            .expect("SVC first")
            .is_empty()
    );
    assert!(calls.lock().expect("backend lock").requests.is_empty());
    assert_response(
        channel
            .process(&svc_frame(20, LAST, &last[9..]))
            .expect("RDPEAR dispatch"),
    );
    assert_eq!(
        calls.lock().expect("backend lock").requests,
        [(vec![0xde, 0xad, 0xbe, 0xef], 4)]
    );
}

fn assert_svc_oversize(error: &PduError) {
    assert!(matches!(error.kind(), PduErrorKind::Decode));
    let source = error
        .source()
        .expect("decode source")
        .downcast_ref::<DecodeError>()
        .expect("structured decode source");
    assert!(matches!(
        source.kind(),
        DecodeErrorKind::InvalidField {
            field: "length",
            reason: "static channel message exceeds configured maximum size",
            offset: Some(8)
        }
    ));
    assert_eq!(
        source.kind().to_string(),
        "invalid `length` at offset 8: static channel message exceeds configured maximum size"
    );
}

#[test]
fn bounded_svc_dispatches_rdpear_only_after_both_layers_complete() {
    let (mut channel, calls) = channel(Some(20));
    dispatch_valid_fragments(&mut channel, &calls);
    let close = encode_vec(&DrdynvcServerPdu::Close(ClosePdu::new(7))).expect("close carrier");
    let responses = channel
        .process(&svc_frame(
            u32::try_from(close.len()).expect("small length"),
            FIRST | LAST,
            &close,
        ))
        .expect("close");
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].encode_unframed_pdu().expect("close response"), [0x40, 7]);
    assert_eq!(calls.lock().expect("backend lock").closes, 1);
    drop(channel);
    assert_eq!(calls.lock().expect("backend lock").closes, 1);
}

#[test]
fn carrier_rejection_recovers_without_disabling_rdpear() {
    for frames in [
        vec![svc_frame(21, FIRST, &[0xaa])],
        vec![svc_frame(20, FIRST | LAST, &[0xaa; 21])],
        vec![svc_frame(20, FIRST, &[0xaa; 21])],
        vec![svc_frame(20, FIRST, &[0xaa; 10]), svc_frame(20, 0, &[0xbb; 11])],
        vec![svc_frame(20, FIRST, &[0xaa; 10]), svc_frame(20, LAST, &[0xbb; 11])],
        vec![svc_frame(21, FIRST | COMPRESSED, &[0xaa])],
        vec![
            svc_frame(2, FIRST | COMPRESSED, &[0xaa; 10]),
            svc_frame(2, LAST | COMPRESSED, &[0xbb; 11]),
        ],
    ] {
        let (mut channel, calls) = channel(Some(20));
        for pending in &frames[..frames.len() - 1] {
            assert!(channel.process(pending).expect("pending SVC").is_empty());
            assert!(calls.lock().expect("backend lock").requests.is_empty());
        }
        let error = channel
            .process(frames.last().expect("bad carrier"))
            .expect_err("SVC cap");
        assert_svc_oversize(&error);
        assert!(calls.lock().expect("backend lock").requests.is_empty());
        assert_eq!(calls.lock().expect("backend lock").closes, 0);
        dispatch_valid_fragments(&mut channel, &calls);
        drop(channel);
        assert_eq!(calls.lock().expect("backend lock").closes, 1);
    }
}

#[test]
fn per_dvc_limit_remains_independent_of_svc_carrier() {
    assert_per_dvc_limit(Some(64));
}

#[test]
fn unlimited_carrier_still_enforces_rdpear_limit() {
    assert_per_dvc_limit(None);
}

fn assert_per_dvc_limit(carrier_limit: Option<usize>) {
    for fragmented in [false, true] {
        let (mut channel, calls) = channel(carrier_limit);
        let pdu = if fragmented {
            DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 29, FRAME[..10].to_vec()))
        } else {
            let mut oversized = FRAME.to_vec();
            oversized[4..8].copy_from_slice(&29u32.to_le_bytes());
            oversized.push(0x99);
            DrdynvcDataPdu::Data(DataPdu::new(7, oversized))
        };
        let bytes = encode_vec(&pdu).expect("oversized DVC carrier");
        assert!(bytes.len() < 64);
        let error = channel
            .process(&svc_frame(
                u32::try_from(bytes.len()).expect("small length"),
                FIRST | LAST,
                &bytes,
            ))
            .expect_err("per-DVC cap");
        assert!(matches!(error.kind(), PduErrorKind::Decode));
        let source = error
            .source()
            .expect("decode source")
            .downcast_ref::<DecodeError>()
            .expect("DVC decode error");
        let expected_field = if fragmented {
            "DataFirstPdu::length"
        } else {
            "DataPdu::data"
        };
        assert!(matches!(
            source.kind(),
            DecodeErrorKind::InvalidField {
                field,
                reason: "DVC message exceeds configured maximum size",
                offset: None
            } if *field == expected_field
        ));
        assert!(calls.lock().expect("backend lock").requests.is_empty());
        assert_eq!(calls.lock().expect("backend lock").closes, 0);
        dispatch_valid_fragments(&mut channel, &calls);
        drop(channel);
        assert_eq!(calls.lock().expect("backend lock").closes, 1);
    }
}

#[test]
fn svc_rejection_preserves_pending_dvc_reassembly() {
    let (mut channel, calls) = channel(Some(20));
    let first = encode_vec(&DrdynvcDataPdu::DataFirst(DataFirstPdu::new(
        7,
        28,
        FRAME[..10].to_vec(),
    )))
    .expect("DVC first");
    assert!(
        channel
            .process(&svc_frame(13, FIRST | LAST, &first))
            .expect("DVC pending")
            .is_empty()
    );
    let error = channel
        .process(&svc_frame(21, FIRST, &[0xaa]))
        .expect_err("carrier cap");
    assert_svc_oversize(&error);
    assert!(calls.lock().expect("backend lock").requests.is_empty());
    let last = encode_vec(&DrdynvcDataPdu::Data(DataPdu::new(7, FRAME[10..].to_vec()))).expect("DVC last");
    assert!(
        channel
            .process(&svc_frame(20, FIRST, &last[..9]))
            .expect("new SVC first")
            .is_empty()
    );
    assert!(calls.lock().expect("backend lock").requests.is_empty());
    assert_response(
        channel
            .process(&svc_frame(20, LAST, &last[9..]))
            .expect("DVC completed"),
    );
    assert_eq!(
        calls.lock().expect("backend lock").requests,
        [(vec![0xde, 0xad, 0xbe, 0xef], 4)]
    );
    drop(channel);
    assert_eq!(calls.lock().expect("backend lock").closes, 1);
}
