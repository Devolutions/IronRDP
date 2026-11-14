use ironrdp_core::{encode_vec, impl_as_any};
use ironrdp_dvc::ironrdp_pdu::PduResult;
use ironrdp_dvc::ironrdp_pdu::rdp::vc::ChannelControlFlags;
use ironrdp_dvc::{DrdynvcServer, DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_svc::{MAX_CHANNEL_CHUNK_LENGTH, StaticChannelSet, SvcProcessor as _};
use rstest::rstest;

use super::*;

#[derive(Default)]
struct RecordedServerDvc {
    received: Vec<Vec<u8>>,
    maximum: Option<usize>,
}

impl_as_any!(RecordedServerDvc);

impl DvcProcessor for RecordedServerDvc {
    fn channel_name(&self) -> &str {
        "recorded"
    }

    fn max_message_size(&self) -> Option<usize> {
        self.maximum
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.received.push(payload.to_vec());
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for RecordedServerDvc {}

fn process_client_pdu(server: &mut DrdynvcServer, pdu: DrdynvcClientPdu) {
    let encoded = encode_vec(&pdu).expect("client PDU should encode");
    server.process(&encoded).expect("client PDU should be processed");
}

fn open_recorded_server(maximum: Option<usize>) -> (DrdynvcServer, u32) {
    open_recorded_server_on(DrdynvcServer::new(), maximum)
}

fn open_recorded_server_on(server: DrdynvcServer, maximum: Option<usize>) -> (DrdynvcServer, u32) {
    let mut server = server.with_dynamic_channel(RecordedServerDvc {
        maximum,
        ..Default::default()
    });
    server.start().expect("server should start");
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    let channel_id = server
        .get_channel_id_by_type::<RecordedServerDvc>()
        .expect("channel should be requested");
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(channel_id, CreationStatus::OK)),
    );
    (server, channel_id)
}

#[test]
fn server_decompresses_client_data() {
    let (mut server, channel_id) = open_recorded_server(None);

    // The MS-RDPEDYC 4.3.3 and 4.3.4 data blocks, sent by the client as two messages: the
    // second matches against the history left by the first.
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirstCompressed(DataFirstCompressedPdu::new(
            channel_id,
            1595,
            vec![0xe0, 0x26, 0x38, 0xc4, 0x3f, 0xf4, 0x74, 0x01],
        ))),
    );
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataCompressed(DataCompressedPdu::new(
            channel_id,
            vec![0xe0, 0x26, 0x88, 0x7f, 0xe8, 0xf4, 0x02],
        ))),
    );

    let channel = server
        .dvc_by_id::<RecordedServerDvc>(channel_id)
        .expect("channel should be open");
    assert_eq!(channel.processor().received, vec![vec![b'q'; 1595], vec![b'q'; 1597]]);
}

// oracle: spec MS-RDPEDYC §2.2.3 and §3.1.5.2.2.
#[test]
fn server_receives_empty_messages_around_fragmented_data() {
    let (mut server, channel_id) = open_recorded_server(None);
    let payloads = [vec![], vec![b'q'; 1591], vec![], vec![b'x'], vec![]];
    let messages = payloads
        .iter()
        .cloned()
        .map(|payload| -> DvcMessage { Box::new(TestPayload(payload)) })
        .collect();
    for message in
        ironrdp_dvc::encode_dvc_messages(channel_id, messages, ironrdp_svc::ChannelFlags::SHOW_PROTOCOL).unwrap()
    {
        server.process(&message.encode_unframed_pdu().unwrap()).unwrap();
    }
    let channel = server.dvc_by_id::<RecordedServerDvc>(channel_id).unwrap();
    assert_eq!(channel.processor().received.as_slice(), payloads.as_slice());
}

// oracle: mutant ending the RDP connection when already-sent data races a server-initiated DVC close.
#[test]
fn server_discards_data_in_flight_when_a_channel_closes() {
    let (mut server, channel_id) = open_recorded_server(None);
    server.close_channel(channel_id).unwrap();
    let late_data = encode_vec(&DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        channel_id,
        b"already in flight".to_vec(),
    ))))
    .unwrap();
    assert!(
        server
            .process(&late_data)
            .expect("late data for a closed DVC must not become an RDP connection error")
            .is_empty()
    );
    let unknown_data = encode_vec(&DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        0x7fff,
        b"never allocated".to_vec(),
    ))))
    .unwrap();
    assert!(
        server.process(&unknown_data).is_err(),
        "an unallocated channel ID was accepted"
    );
}

// oracle: mutant ending the desktop when a timed-out creation receives a late response.
#[test]
fn cancelled_creation_can_be_reopened_without_reusing_the_channel_id() {
    let mut server = DrdynvcServer::new();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    let request = server.create_channel(RecordedServerDvc::default()).unwrap();
    let DrdynvcServerPdu::Create(request) = ironrdp_core::decode(&request.encode_unframed_pdu().unwrap()).unwrap()
    else {
        panic!("expected a create request");
    };
    let retired_id = request.channel_id();
    server.close_channel(retired_id).unwrap();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(retired_id, CreationStatus::OK)),
    );
    let request = server.create_channel(RecordedServerDvc::default()).unwrap();
    let DrdynvcServerPdu::Create(request) = ironrdp_core::decode(&request.encode_unframed_pdu().unwrap()).unwrap()
    else {
        panic!("expected a create request");
    };
    assert_ne!(request.channel_id(), retired_id);
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(request.channel_id(), CreationStatus::OK)),
    );
    assert!(server.is_channel_opened(request.channel_id()));
}

// oracle: spec MS-RDPEDYC §2.2.4; mutant emitting ClosePdu for Pending or CreationFailed channels.
#[test]
fn close_channel_suppresses_pdu_for_pending_and_declined_channels() {
    let mut server = DrdynvcServer::new().with_dynamic_channel(RecordedServerDvc::default());
    let pending_id = server.get_channel_id_by_type::<RecordedServerDvc>().unwrap();
    assert!(server.close_channel(pending_id).is_none());
    let cap_responses = server
        .process(
            &encode_vec(&DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(
                CapsVersion::V3,
            )))
            .unwrap(),
        )
        .unwrap();
    assert!(cap_responses.is_empty());

    let request = server.create_channel(RecordedServerDvc::default()).unwrap();
    let DrdynvcServerPdu::Create(request) = ironrdp_core::decode(&request.encode_unframed_pdu().unwrap()).unwrap()
    else {
        panic!("expected a create request");
    };
    let declined_id = request.channel_id();
    assert_ne!(declined_id, pending_id);
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(declined_id, CreationStatus::NO_LISTENER)),
    );
    assert!(server.close_channel(declined_id).is_none());
}

#[rstest]
// oracle: mutant buffering an oversized fragmented message before the backend can reject it.
#[case::processor_limit(DrdynvcServer::new(), Some(8))]
// oracle: mutant ignoring the server-wide message limit for a channel whose processor sets none.
#[case::server_limit(DrdynvcServer::new().with_max_message_size(8), None)]
fn server_rejects_oversized_fragmented_messages_before_reassembly(
    #[case] server: DrdynvcServer,
    #[case] maximum: Option<usize>,
) {
    let (mut server, id) = open_recorded_server_on(server, maximum);
    let response = server
        .process(
            &encode_vec(&DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirst(DataFirstPdu::new(
                id,
                1_000_000_000,
                vec![1],
            ))))
            .unwrap(),
        )
        .unwrap();
    assert!(!server.is_channel_opened(id));
    assert_eq!(response.len(), 1);
    assert_eq!(
        ironrdp_core::decode::<DrdynvcServerPdu>(&response[0].encode_unframed_pdu().unwrap()).unwrap(),
        DrdynvcServerPdu::Close(ClosePdu::new(id))
    );
}

// oracle: mutant refusing a message whose declared length equals the server-wide limit.
#[test]
fn a_message_declared_at_the_limit_is_delivered() {
    let (mut server, id) = open_recorded_server_on(DrdynvcServer::new().with_max_message_size(8), None);
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirst(DataFirstPdu::new(id, 8, vec![1; 4]))),
    );
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(id, vec![1; 4]))),
    );
    assert!(server.is_channel_opened(id));
    let channel = server.dvc_by_id::<RecordedServerDvc>(id).unwrap();
    assert_eq!(channel.processor().received, vec![vec![1; 8]]);
}

// oracle: mutant applying the server-wide message limit over a processor's own.
#[test]
fn a_processor_limit_takes_precedence_over_the_server_limit() {
    let (mut server, id) = open_recorded_server_on(DrdynvcServer::new().with_max_message_size(8), Some(16));
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(id, vec![1; 12]))),
    );
    assert!(server.is_channel_opened(id));
    let channel = server.dvc_by_id::<RecordedServerDvc>(id).unwrap();
    assert_eq!(channel.processor().received, vec![vec![1; 12]]);
}

// oracle: mutant taking the larger of a processor's own message limit and the server-wide one.
#[test]
fn a_stricter_processor_limit_takes_precedence_over_the_server_limit() {
    let (mut server, id) = open_recorded_server_on(DrdynvcServer::new().with_max_message_size(16), Some(8));
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(id, vec![1; 12]))),
    );
    assert!(!server.is_channel_opened(id));
}

/// A message from the client as one static channel chunk.
fn static_chunk(pdu: &DrdynvcClientPdu) -> Vec<u8> {
    let body = encode_vec(pdu).unwrap();
    crate::svc::channel_chunk(
        &body,
        u32::try_from(body.len()).unwrap(),
        ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
    )
}

/// The first static channel chunk of a message declared `length` bytes long.
fn first_chunk(length: u32) -> Vec<u8> {
    crate::svc::channel_chunk(&[1; 8], length, ChannelControlFlags::FLAG_FIRST)
}

/// The limit of the static channel set in which [`open_in_bounded_static_set`] puts the DRDYNVC static channel.
const STATIC_LIMIT: usize = 15;

/// The recorded channel, opened on `server`, which is then put in a static channel set bounded at [`STATIC_LIMIT`].
fn open_in_bounded_static_set(server: DrdynvcServer) -> (StaticChannelSet, u32) {
    let (server, id) = open_recorded_server_on(server, None);
    let mut set = StaticChannelSet::new();
    set.set_max_message_size(STATIC_LIMIT);
    set.insert(server);
    (set, id)
}

// oracle: mutant bounding the DRDYNVC static channel at the dynamic channel limit, which leaves no room for the DRDYNVC
// header.
#[test]
fn a_message_at_the_limit_passes_through_a_bounded_static_set() {
    let (mut set, id) = open_in_bounded_static_set(DrdynvcServer::new().with_max_message_size(15));
    let svc = set.get_by_type_mut::<DrdynvcServer>().unwrap();
    svc.process(&static_chunk(&DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(
        DataPdu::new(id, vec![1; 15]),
    ))))
    .expect("a message at the limit");
    let server = svc.channel_processor_downcast_ref::<DrdynvcServer>().unwrap();
    assert!(server.is_channel_opened(id));
    assert_eq!(
        server.dvc_by_id::<RecordedServerDvc>(id).unwrap().processor().received,
        vec![vec![1; 15]]
    );
}

// oracle: mutant ignoring the server-wide message limit for an unfragmented message.
#[test]
fn a_message_over_the_limit_closes_only_its_channel_through_a_bounded_static_set() {
    let (mut set, id) = open_in_bounded_static_set(DrdynvcServer::new().with_max_message_size(15));
    let svc = set.get_by_type_mut::<DrdynvcServer>().unwrap();
    svc.process(&static_chunk(&DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(
        DataPdu::new(id, vec![1; 16]),
    ))))
    .expect("an oversized message closes only its channel");
    assert!(
        !svc.channel_processor_downcast_ref::<DrdynvcServer>()
            .unwrap()
            .is_channel_opened(id)
    );
}

// oracle: spec MS-RDPEDYC §2.2.3.1, a DYNVC_DATA_FIRST PDU may fill 1,600 bytes; mutant bounding the DRDYNVC static
// channel below that for a small dynamic channel limit, which ends the session.
#[test]
fn a_full_size_pdu_passes_a_static_channel_bounded_below_it() {
    const CHANNEL_PDU_HEADER_SIZE: usize = 4 /* length */ + 4 /* flags */;
    const DATA_FIRST_HEADER_SIZE: usize = 1 /* cbId, Len, Cmd */ + 1 /* ChannelId */ + 4 /* Length */;

    let (mut set, id) = open_in_bounded_static_set(DrdynvcServer::new().with_max_message_size(15));
    let chunk = static_chunk(&DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirst(DataFirstPdu::new(
        id,
        1_000_000,
        vec![1; 1600 - DATA_FIRST_HEADER_SIZE],
    ))));
    assert_eq!(chunk.len(), CHANNEL_PDU_HEADER_SIZE + 1600, "a full-size PDU");
    let svc = set.get_by_type_mut::<DrdynvcServer>().unwrap();
    svc.process(&chunk).expect("a full-size PDU closes only its channel");
    assert!(
        !svc.channel_processor_downcast_ref::<DrdynvcServer>()
            .unwrap()
            .is_channel_opened(id)
    );
}

#[rstest]
// oracle: mutant leaving the DRDYNVC static channel unbounded without a dynamic channel limit, which lets a client make
// it keep a declared 4 GiB under a static channel set's limit.
#[case::static_set_limit(DrdynvcServer::new, STATIC_LIMIT)]
// oracle: mutant leaving the DRDYNVC static channel unbounded with a dynamic channel limit, or bounding it elsewhere
// than at the limit plus one chunk.
#[case::dynamic_limit_plus_one_chunk(|| DrdynvcServer::new().with_max_message_size(15), 15 + MAX_CHANNEL_CHUNK_LENGTH)]
fn the_drdynvc_static_channel_is_bounded(#[case] server: fn() -> DrdynvcServer, #[case] bound: usize) {
    let bound = u32::try_from(bound).unwrap();

    let (mut set, _) = open_in_bounded_static_set(server());
    let svc = set.get_by_type_mut::<DrdynvcServer>().unwrap();
    svc.process(&first_chunk(bound))
        .expect("a message declared at the bound");

    let (mut set, _) = open_in_bounded_static_set(server());
    let svc = set.get_by_type_mut::<DrdynvcServer>().unwrap();
    let error = svc
        .process(&first_chunk(bound + 1))
        .expect_err("a message declared over the bound");
    assert!(
        format!("{error:?}").contains("exceeds configured maximum size"),
        "refused for another reason: {error:?}"
    );
}

// oracle: mutant wrapping the DRDYNVC static channel's bound for a dynamic channel limit near `usize::MAX`, which
// bounds it below a full-size PDU.
#[test]
fn the_largest_dynamic_channel_limit_leaves_drdynvc_unbounded() {
    let (mut set, _) = open_in_bounded_static_set(DrdynvcServer::new().with_max_message_size(usize::MAX));
    let svc = set.get_by_type_mut::<DrdynvcServer>().unwrap();
    svc.process(&first_chunk(u32::MAX))
        .expect("a message declared at the largest length");
}

// oracle: mutant ignoring DataFirstCompressedPdu::Length when bounding a message.
#[test]
fn a_declared_compressed_length_over_the_limit_closes_the_channel() {
    let (mut server, id) = open_recorded_server_on(DrdynvcServer::new().with_max_message_size(8), None);
    // The MS-RDPEDYC 4.3.3 block, which decompresses to 1595 bytes, as the start of a far longer message: only the
    // declared length can refuse it.
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirstCompressed(DataFirstCompressedPdu::new(
            id,
            1_000_000,
            vec![0xe0, 0x26, 0x38, 0xc4, 0x3f, 0xf4, 0x74, 0x01],
        ))),
    );
    assert!(!server.is_channel_opened(id));
}

// oracle: mutant enforcing the backend message limit only for fragmented data.
#[test]
fn server_rejects_oversized_unfragmented_messages() {
    let (mut server, id) = open_recorded_server(Some(8));
    let response = server
        .process(
            &encode_vec(&DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
                id,
                vec![1; 9],
            ))))
            .unwrap(),
        )
        .unwrap();
    assert!(!server.is_channel_opened(id));
    assert_eq!(response.len(), 1);
    assert_eq!(
        ironrdp_core::decode::<DrdynvcServerPdu>(&response[0].encode_unframed_pdu().unwrap()).unwrap(),
        DrdynvcServerPdu::Close(ClosePdu::new(id))
    );

    let (mut server, compressed_id) = open_recorded_server(Some(8));
    let compressed_response = server
        .process(
            &encode_vec(&DrdynvcClientPdu::Data(DrdynvcDataPdu::DataCompressed(
                DataCompressedPdu::new(compressed_id, vec![0xe0, 0x26, 0x38, 0xc4, 0x3f, 0xf4, 0x74, 0x01]),
            )))
            .unwrap(),
        )
        .unwrap();
    assert!(!server.is_channel_opened(compressed_id));
    assert_eq!(compressed_response.len(), 1);
    assert_eq!(
        ironrdp_core::decode::<DrdynvcServerPdu>(&compressed_response[0].encode_unframed_pdu().unwrap()).unwrap(),
        DrdynvcServerPdu::Close(ClosePdu::new(compressed_id))
    );
}

// oracle: spec MS-RDPEDYC §3.1.5.2.3; mutant retaining fragments that overrun DataFirstPdu::Length.
#[test]
fn server_rejects_fragments_exceeding_declared_message_length() {
    let (mut server, id) = open_recorded_server(None);
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirst(DataFirstPdu::new(id, 4, vec![1, 2]))),
    );
    let overrun = encode_vec(&DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        id,
        vec![3, 4, 5],
    ))))
    .unwrap();
    assert!(server.process(&overrun).is_err());
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(id, vec![9]))),
    );
    let channel = server.dvc_by_id::<RecordedServerDvc>(id).unwrap();
    assert_eq!(channel.processor().received, vec![vec![9]]);
}

// oracle: spec MS-RDPEDYC §1.3.1 and §3.3.5.1.
#[test]
fn server_tracks_readiness_and_creates_boxed_dynamic_channel() {
    let mut server = DrdynvcServer::new();
    assert!(!server.is_ready());
    assert!(
        server
            .create_channel_boxed_with(Box::new(|_| Box::new(RecordedServerDvc::default())))
            .is_err()
    );
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    assert!(server.is_ready());
    let request = server
        .create_channel_boxed_with(Box::new(|id| {
            Box::new(RecordedServerDvc {
                received: vec![id.to_le_bytes().to_vec()],
                ..Default::default()
            })
        }))
        .unwrap();
    let DrdynvcServerPdu::Create(request) = ironrdp_core::decode(&request.encode_unframed_pdu().unwrap()).unwrap()
    else {
        panic!("expected a create request");
    };
    let id = request.channel_id();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::OK)),
    );
    let channel = server.dvc_by_id::<RecordedServerDvc>(id).unwrap();
    assert_eq!(channel.processor().received, vec![id.to_le_bytes().to_vec()]);
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(32))]
    // oracle: property round-trip.
    #[test]
    fn encode_dvc_messages_round_trips_through_server_prop(
        lengths in proptest::collection::vec(0..=(DrdynvcDataPdu::MAX_DATA_SIZE * 3 + 8), 1..6usize)
    ) {
        let (mut server, channel_id) = open_recorded_server(None);
        let payloads: Vec<Vec<u8>> = lengths
            .into_iter()
            .enumerate()
            .map(|(idx, len)| vec![u8::try_from(idx).unwrap_or(0); len])
            .collect();
        let messages = payloads
            .iter()
            .cloned()
            .map(|payload| -> DvcMessage { Box::new(TestPayload(payload)) })
            .collect();
        for message in
            ironrdp_dvc::encode_dvc_messages(channel_id, messages, ironrdp_svc::ChannelFlags::SHOW_PROTOCOL).unwrap()
        {
            let wire = message.encode_unframed_pdu().unwrap();
            let decoded = ironrdp_core::decode::<DrdynvcServerPdu>(&wire).unwrap();
            match &decoded {
                DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) => {
                    proptest::prop_assert!(data.data().len() <= DrdynvcDataPdu::MAX_DATA_SIZE);
                }
                DrdynvcServerPdu::Data(DrdynvcDataPdu::DataFirst(first)) => {
                    proptest::prop_assert!(first.data().len() <= DrdynvcDataPdu::MAX_DATA_SIZE);
                    proptest::prop_assert!(DrdynvcDataPdu::MAX_DATA_SIZE < usize::try_from(first.length()).unwrap());
                }
                other => panic!("unexpected DVC PDU: {other:?}"),
            }
            server.process(&wire).unwrap();
        }
        let channel = server.dvc_by_id::<RecordedServerDvc>(channel_id).unwrap();
        proptest::prop_assert_eq!(channel.processor().received.as_slice(), payloads.as_slice());
    }
}
