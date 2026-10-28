use ironrdp_core::{encode_vec, impl_as_any};
use ironrdp_dvc::ironrdp_pdu::PduResult;
use ironrdp_dvc::{DrdynvcServer, DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_svc::SvcProcessor as _;

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

#[test]
fn server_decompresses_client_data() {
    let mut server = DrdynvcServer::new().with_dynamic_channel(RecordedServerDvc::default());
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
    let mut server = DrdynvcServer::new().with_dynamic_channel(RecordedServerDvc::default());
    server.start().unwrap();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    let channel_id = server.get_channel_id_by_type::<RecordedServerDvc>().unwrap();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(channel_id, CreationStatus::OK)),
    );
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
    let mut server = DrdynvcServer::new().with_dynamic_channel(RecordedServerDvc::default());
    server.start().unwrap();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    let channel_id = server.get_channel_id_by_type::<RecordedServerDvc>().unwrap();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(channel_id, CreationStatus::OK)),
    );
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

// oracle: mutant buffering an oversized fragmented message before the backend can reject it.
#[test]
fn server_rejects_oversized_fragmented_messages_before_reassembly() {
    let mut server = DrdynvcServer::new().with_dynamic_channel(RecordedServerDvc {
        maximum: Some(8),
        ..Default::default()
    });
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    let id = server.get_channel_id_by_type::<RecordedServerDvc>().unwrap();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::OK)),
    );
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

// oracle: mutant enforcing the backend message limit only for fragmented data.
#[test]
fn server_rejects_oversized_unfragmented_messages() {
    let mut server = DrdynvcServer::new().with_dynamic_channel(RecordedServerDvc {
        maximum: Some(8),
        ..Default::default()
    });
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    let id = server.get_channel_id_by_type::<RecordedServerDvc>().unwrap();
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::OK)),
    );
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
}
