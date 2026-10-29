use std::sync::{Arc, Mutex};

use ironrdp_core::{Decode as _, Encode as _, ReadCursor, WriteCursor, encode_vec};
use ironrdp_dvc::pdu::{
    ClosePdu, CreateRequestPdu, DataFirstPdu, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu, DrdynvcServerPdu,
};
use ironrdp_dvc::{DrdynvcClient, DvcProcessor as _};
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpear::{AuthRedirectionBackend, AuthRedirectionClient, AuthRedirectionPdu, CHANNEL_NAME};
use ironrdp_svc::SvcProcessor as _;

const FRAME: &[u8] = &[
    0xc8, 0xc3, 0xac, 0x4e, // ProtocolMagic
    4, 0, 0, 0, // encrypted payload Length
    0, 0, 0, 0, // Version
    0, 0, 0, 0, // Reserved
    0, 0, 0, 0, 0, 0, 0, 0, // TsPkgContext
    0xde, 0xad, 0xbe, 0xef, // opaque ciphertext, not an authentication fixture
];

#[derive(Default)]
struct Calls {
    requests: Vec<(Vec<u8>, usize)>,
    closes: usize,
}

struct Backend {
    calls: Arc<Mutex<Calls>>,
    response: Vec<u8>,
    fail: bool,
}

impl AuthRedirectionBackend for Backend {
    fn process(&mut self, encrypted_payload: &[u8], max_payload_size: usize) -> PduResult<Vec<u8>> {
        self.calls
            .lock()
            .expect("test lock")
            .requests
            .push((encrypted_payload.to_vec(), max_payload_size));
        if self.fail {
            return Err(pdu_other_err!("test security context rejected request"));
        }
        Ok(self.response.clone())
    }

    fn close(&mut self) {
        self.calls.lock().expect("test lock").closes += 1;
    }
}

fn client(limit: u32, response: Vec<u8>, fail: bool) -> (AuthRedirectionClient, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let backend = Backend {
        calls: Arc::clone(&calls),
        response,
        fail,
    };
    (
        AuthRedirectionClient::new(Box::new(backend), limit).expect("valid limit"),
        calls,
    )
}

#[test]
fn outer_frame_matches_wire_fixture() {
    let mut cursor = ReadCursor::new(FRAME);
    let pdu = AuthRedirectionPdu::decode(&mut cursor).expect("valid frame");
    assert!(cursor.is_empty());
    assert_eq!(pdu.reserved, 0);
    assert_eq!(pdu.encrypted_payload, [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(pdu.size(), FRAME.len());
    assert_eq!(encode_vec(&pdu).expect("encode frame"), FRAME);
}

#[test]
fn native_payload_length_geometry_rejects_truncated_envelopes() {
    // accepted1 frames 128/129: only public header/length metadata, with synthetic payloads.
    for (header, total_length) in [
        (
            [
                0xc8, 0xc3, 0xac, 0x4e, 134, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            158,
        ),
        (
            [
                0xc8, 0xc3, 0xac, 0x4e, 142, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            166,
        ),
    ] {
        let mut frame = header.to_vec();
        frame.resize(total_length, 0x5a);
        let mut cursor = ReadCursor::new(&frame);
        let pdu = AuthRedirectionPdu::decode(&mut cursor).expect("native payload-length geometry");
        assert!(cursor.is_empty());
        assert_eq!(pdu.encrypted_payload, vec![0x5a; total_length - 24]);
        assert_eq!(encode_vec(&pdu).expect("encode native geometry"), frame);
        assert!(
            AuthRedirectionPdu::decode(&mut ReadCursor::new(&frame[..total_length - 24])).is_err(),
            "a 24-byte-short envelope must not be reinterpreted as total-length framing"
        );
        for end in 0..frame.len() {
            assert!(
                AuthRedirectionPdu::decode(&mut ReadCursor::new(&frame[..end])).is_err(),
                "total {total_length}, prefix {end}"
            );
        }
        let mut trailing = frame.clone();
        trailing.push(0);
        assert!(AuthRedirectionPdu::decode(&mut ReadCursor::new(&trailing)).is_err());
        let mut total_length_form = frame;
        total_length_form[4..8].copy_from_slice(&u32::try_from(total_length).expect("small length").to_le_bytes());
        assert!(AuthRedirectionPdu::decode(&mut ReadCursor::new(&total_length_form)).is_err());
    }
}

#[test]
fn reserved_field_is_preserved_not_rejected() {
    let mut frame = FRAME.to_vec();
    frame[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    let pdu = AuthRedirectionPdu::decode(&mut ReadCursor::new(&frame)).expect("reserved is not constrained");
    assert_eq!(pdu.reserved, u32::MAX);
    assert_eq!(encode_vec(&pdu).expect("encode frame"), frame);
}

#[test]
fn rejects_every_truncated_frame_and_trailing_bytes() {
    for end in 0..FRAME.len() {
        assert!(
            AuthRedirectionPdu::decode(&mut ReadCursor::new(&FRAME[..end])).is_err(),
            "prefix {end}"
        );
    }
    let mut frame = FRAME.to_vec();
    frame.push(0);
    assert!(AuthRedirectionPdu::decode(&mut ReadCursor::new(&frame)).is_err());
}

#[test]
fn rejects_invalid_header_fields() {
    for (offset, bytes) in [
        (0, 0u32.to_le_bytes()),
        (4, 23u32.to_le_bytes()),
        (4, 28u32.to_le_bytes()),
        (4, 29u32.to_le_bytes()),
        (4, u32::MAX.to_le_bytes()),
        (8, 1u32.to_le_bytes()),
        (16, 1u32.to_le_bytes()),
        (20, 1u32.to_le_bytes()),
    ] {
        let mut frame = FRAME.to_vec();
        frame[offset..offset + 4].copy_from_slice(&bytes);
        assert!(
            AuthRedirectionPdu::decode(&mut ReadCursor::new(&frame)).is_err(),
            "field at {offset}"
        );
    }
}

#[test]
fn rejects_empty_payload_and_short_encode_buffer() {
    let mut frame = FRAME[..24].to_vec();
    frame[4..8].copy_from_slice(&0u32.to_le_bytes());
    assert!(AuthRedirectionPdu::decode(&mut ReadCursor::new(&frame)).is_err());
    let empty = AuthRedirectionPdu {
        reserved: 0,
        encrypted_payload: Vec::new(),
    };
    assert!(encode_vec(&empty).is_err());
    let pdu = AuthRedirectionPdu::decode(&mut ReadCursor::new(FRAME)).expect("valid frame");
    let mut short = [0; 27];
    assert!(pdu.encode(&mut WriteCursor::new(&mut short)).is_err());
}

#[test]
fn ciphertext_is_not_in_debug_output() {
    let pdu = AuthRedirectionPdu::decode(&mut ReadCursor::new(FRAME)).expect("valid frame");
    assert_eq!(
        format!("{pdu:?}"),
        "AuthRedirectionPdu { reserved: 0, encrypted_payload_length: 4 }"
    );
}

#[test]
fn client_dispatches_ciphertext_and_frames_response_at_exact_limit() {
    let (mut client, calls) = client(28, vec![1, 2, 3, 4], false);
    assert_eq!(client.channel_name(), CHANNEL_NAME);
    assert!(client.start(7).expect("start").is_empty());
    let mut request = FRAME.to_vec();
    request[12] = 1;
    for _ in 0..2 {
        let responses = client.process(7, &request).expect("dispatch");
        assert_eq!(responses.len(), 1);
        let encoded = encode_vec(responses[0].as_ref()).expect("response");
        let mut expected = FRAME.to_vec();
        expected[24..].copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(encoded, expected);
    }
    assert_eq!(
        calls.lock().expect("test lock").requests,
        vec![(FRAME[24..].to_vec(), 4); 2]
    );
}

#[test]
fn invalid_or_oversized_requests_never_reach_backend_and_disable_processing() {
    for request in [vec![0; 28], vec![0; 29]] {
        let (mut client, calls) = client(28, vec![1], false);
        client.start(7).expect("start");
        assert!(client.process(7, &request).is_err());
        assert!(client.process(7, FRAME).is_err());
        assert!(calls.lock().expect("test lock").requests.is_empty());
        client.close(7);
        assert_eq!(calls.lock().expect("test lock").closes, 1);
    }
}

#[test]
fn backend_errors_and_invalid_responses_disable_processing() {
    for (response, fail) in [(vec![1], true), (Vec::new(), false), (vec![1; 5], false)] {
        let (mut client, calls) = client(28, response, fail);
        client.start(7).expect("start");
        assert!(client.process(7, FRAME).is_err());
        assert!(client.process(7, FRAME).is_err());
        assert_eq!(calls.lock().expect("test lock").requests.len(), 1);
        assert!(client.start(7).is_err());
        client.close(7);
        assert_eq!(calls.lock().expect("test lock").closes, 1);
    }
}

#[test]
fn channel_lifecycle_is_single_use_and_bound_to_channel_id() {
    let (mut client, calls) = client(28, vec![1], false);
    assert!(client.process(7, FRAME).is_err());
    client.close(7);
    assert_eq!(calls.lock().expect("test lock").closes, 0);
    client.start(7).expect("start");
    assert!(client.start(8).is_err());
    assert!(client.process(8, FRAME).is_err());
    client.close(8);
    assert_eq!(calls.lock().expect("test lock").closes, 0);
    assert_eq!(client.process(7, FRAME).expect("still active").len(), 1);
    client.close(7);
    client.close(7);
    assert_eq!(calls.lock().expect("test lock").closes, 1);
    assert!(client.process(7, FRAME).is_err());
    assert!(client.start(7).is_err());
    assert_eq!(calls.lock().expect("test lock").requests.len(), 1);
}

#[test]
fn constructor_requires_room_for_ciphertext() {
    for limit in [0, 23, 24] {
        let backend = Backend {
            calls: Arc::new(Mutex::new(Calls::default())),
            response: vec![1],
            fail: false,
        };
        assert!(AuthRedirectionClient::new(Box::new(backend), limit).is_err());
    }
    let (mut client, calls) = client(25, vec![1], false);
    client.start(7).expect("start");
    let pdu = AuthRedirectionPdu {
        reserved: 0,
        encrypted_payload: vec![2],
    };
    assert_eq!(
        client
            .process(7, &encode_vec(&pdu).expect("encode"))
            .expect("minimum message")
            .len(),
        1
    );
    assert_eq!(calls.lock().expect("test lock").requests, vec![(vec![2], 1)]);
}

#[test]
fn drdynvc_reassembles_request_routes_response_and_closes_backend() {
    let (client, calls) = client(28, vec![1], false);
    let mut drdynvc = DrdynvcClient::new().with_dynamic_channel(client);
    let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(7, CHANNEL_NAME.to_owned()));
    let responses = drdynvc
        .process(&encode_vec(&create).expect("create"))
        .expect("create channel");
    assert_eq!(responses.len(), 2); // implicit DVC capabilities and create response only
    assert!(calls.lock().expect("test lock").requests.is_empty());

    let first = DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 28, FRAME[..10].to_vec()));
    assert!(
        drdynvc
            .process(&encode_vec(&first).expect("first fragment"))
            .expect("reassemble")
            .is_empty()
    );
    assert!(calls.lock().expect("test lock").requests.is_empty());
    let last = DrdynvcDataPdu::Data(DataPdu::new(7, FRAME[10..].to_vec()));
    let responses = drdynvc
        .process(&encode_vec(&last).expect("last fragment"))
        .expect("dispatch");
    assert_eq!(responses.len(), 1);
    let response = responses[0].encode_unframed_pdu().expect("DVC response");
    let response = DrdynvcClientPdu::decode(&mut ReadCursor::new(&response)).expect("decode DVC response");
    let DrdynvcClientPdu::Data(response) = response else {
        unreachable!("expected DVC data");
    };
    assert_eq!(response.channel_id(), 7);
    let DrdynvcDataPdu::Data(response) = response else {
        unreachable!("expected unfragmented DVC response");
    };
    let pdu = AuthRedirectionPdu::decode(&mut ReadCursor::new(response.data())).expect("outer response");
    assert_eq!(pdu.encrypted_payload, [1]);
    assert_eq!(
        calls.lock().expect("test lock").requests,
        vec![(FRAME[24..].to_vec(), 4)]
    );

    let close = DrdynvcServerPdu::Close(ClosePdu::new(7));
    let _ = drdynvc
        .process(&encode_vec(&close).expect("close"))
        .expect("close channel");
    assert_eq!(calls.lock().expect("test lock").closes, 1);
    drop(drdynvc);
    assert_eq!(calls.lock().expect("test lock").closes, 1);
}

#[test]
fn drdynvc_drop_releases_backend_and_processing_errors_propagate() {
    let (client, calls) = client(28, vec![1], true);
    let mut drdynvc = DrdynvcClient::new().with_dynamic_channel(client);
    let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(7, CHANNEL_NAME.to_owned()));
    let _ = drdynvc
        .process(&encode_vec(&create).expect("create"))
        .expect("create channel");
    let data = DrdynvcDataPdu::Data(DataPdu::new(7, FRAME.to_vec()));
    let data = encode_vec(&data).expect("encode DVC data");
    assert!(drdynvc.process(&data).is_err());
    assert!(drdynvc.process(&data).is_err());
    assert_eq!(calls.lock().expect("test lock").requests.len(), 1);
    drop(drdynvc);
    assert_eq!(calls.lock().expect("test lock").closes, 1);
}

#[test]
fn drdynvc_rejects_oversized_advertised_request_before_backend() {
    let (client, calls) = client(28, vec![1], false);
    let mut drdynvc = DrdynvcClient::new().with_dynamic_channel(client);
    let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(7, CHANNEL_NAME.to_owned()));
    drdynvc
        .process(&encode_vec(&create).expect("create"))
        .expect("create channel");

    let first = DrdynvcDataPdu::DataFirst(DataFirstPdu::new(7, 29, FRAME[..10].to_vec()));
    assert!(drdynvc.process(&encode_vec(&first).expect("first fragment")).is_err());
    assert!(calls.lock().expect("test lock").requests.is_empty());
}

#[test]
fn completed_oversized_request_never_reaches_backend() {
    let (mut client, calls) = client(28, vec![1], false);
    client.start(7).expect("start");

    let mut oversized_request = FRAME.to_vec();
    oversized_request.push(0xaa);
    assert!(client.process(7, &oversized_request).is_err());
    assert!(calls.lock().expect("test lock").requests.is_empty());
}
