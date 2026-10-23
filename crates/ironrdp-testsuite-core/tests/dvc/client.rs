use ironrdp_core::{Decode as _, ReadCursor, encode_vec, impl_as_any};
use ironrdp_dvc::ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_dvc::pdu::{
    CapabilitiesRequestPdu, CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateRequestPdu, CreationStatus,
    DataCompressedPdu, DataFirstCompressedPdu, DataFirstPdu, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu,
    DrdynvcServerPdu, SoftSyncChannelList, SoftSyncRequestPdu, SoftSyncTunnelType,
};
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DvcMessage, DvcMessageBatch, DvcProcessor};
use ironrdp_svc::SvcMessage;
use ironrdp_svc::SvcProcessor as _;

#[derive(Default)]
struct RecordedDvc {
    started_with: Option<u32>,
    received: Vec<Vec<u8>>,
}

impl_as_any!(RecordedDvc);

impl DvcProcessor for RecordedDvc {
    fn channel_name(&self) -> &str {
        "recorded"
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.started_with = Some(channel_id);
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.received.push(payload.to_vec());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for RecordedDvc {}

struct FailingDvc;

impl_as_any!(FailingDvc);

impl DvcProcessor for FailingDvc {
    fn channel_name(&self) -> &str {
        "failing"
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Err(pdu_other_err!("failed to restore dynamic channel"))
    }

    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for FailingDvc {}

struct TunnelCreatedDvc;

impl_as_any!(TunnelCreatedDvc);

impl DvcProcessor for TunnelCreatedDvc {
    fn channel_name(&self) -> &str {
        "tunnel-created"
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for TunnelCreatedDvc {}

fn decode_client_pdu(message: &SvcMessage) -> DrdynvcClientPdu {
    let encoded = message.encode_unframed_pdu().expect("DVC response should encode");
    DrdynvcClientPdu::decode(&mut ReadCursor::new(&encoded)).expect("DVC response should decode")
}

fn exchange_capabilities(client: &mut DrdynvcClient) {
    let caps = encode_vec(&DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(
        CapsVersion::V3,
        None,
    )))
    .expect("Capabilities Request should encode");
    client.process(&caps).expect("Capabilities Request should be processed");
}

#[test]
fn established_dynamic_channel_routes_recorded_data_without_negotiation() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");

    let payload = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        7,
        b"recorded data".to_vec(),
    ))))
    .expect("recorded DVC data should encode");
    assert!(
        client
            .process(&payload)
            .expect("recorded DVC data should route")
            .is_empty()
    );

    let channel = client
        .get_dvc_by_channel_id::<RecordedDvc>(7)
        .expect("recorded channel should remain attached");
    assert_eq!(channel.processor().started_with, Some(7));
    assert_eq!(channel.processor().received, vec![b"recorded data".to_vec()]);
}

#[test]
fn established_dynamic_channel_rejects_repeated_ids() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("first attachment should succeed");

    let error = client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect_err("duplicate channel ID should fail");

    assert!(error.to_string().contains("dynamic channel ID is already attached"));
}

#[test]
fn failed_established_dynamic_channel_attachment_is_not_registered() {
    let mut client = DrdynvcClient::new();

    let error = client
        .attach_established_dynamic_channel(7, FailingDvc)
        .expect_err("failing channel should not attach");

    assert!(error.to_string().contains("failed to restore dynamic channel"));
    assert!(client.get_dvc_by_channel_id::<FailingDvc>(7).is_none());
    assert!(
        client
            .attach_established_dynamic_channel(7, RecordedDvc::default())
            .is_ok()
    );
}

#[test]
fn capabilities_response_echoes_the_offered_version() {
    for version in [CapsVersion::V1, CapsVersion::V2, CapsVersion::V3] {
        let mut client = DrdynvcClient::new();
        let request = encode_vec(&DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(
            version, None,
        )))
        .expect("Capabilities Request should encode");

        let responses = client
            .process(&request)
            .expect("Capabilities Request should be processed");
        let [response] = responses.as_slice() else {
            panic!("expected exactly one Capabilities Response");
        };
        assert_eq!(
            decode_client_pdu(response),
            DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(version)),
            "server offered {version:?}"
        );
    }
}

#[test]
fn compressed_and_uncompressed_blocks_reassemble_into_one_message() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(3, RecordedDvc::default())
        .expect("recorded channel should attach");

    // MS-RDPEDYC 4.3.3 and 4.3.4, byte for byte. The first block decompresses to 1,595 bytes
    // and the second, which matches against the first, to 1,597. The sample declares 3,195
    // bytes in total, so an uncompressed DYNVC_DATA block supplies the last three.
    let data_first_compressed = [0x64, 0x03, 0x7b, 0x0c, 0xe0, 0x26, 0x38, 0xc4, 0x3f, 0xf4, 0x74, 0x01];
    let data_compressed = [0x70, 0x03, 0xe0, 0x26, 0x88, 0x7f, 0xe8, 0xf4, 0x02];
    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        3,
        b"qqq".to_vec(),
    ))))
    .expect("DVC data should encode");

    for pdu in [&data_first_compressed[..], &data_compressed[..], data.as_slice()] {
        assert!(client.process(pdu).expect("DVC data should be processed").is_empty());
    }

    let channel = client
        .get_dvc_by_channel_id::<RecordedDvc>(3)
        .expect("recorded channel should remain attached");
    assert_eq!(channel.processor().received, vec![vec![b'q'; 3195]]);
}

#[test]
fn each_channel_has_its_own_decompression_history() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    for channel_id in [3, 4] {
        client
            .attach_established_dynamic_channel(channel_id, RecordedDvc::default())
            .expect("recorded channel should attach");
    }

    let data_compressed = |channel_id, data: &[u8]| {
        encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::DataCompressed(
            DataCompressedPdu::new(channel_id, data.to_vec()),
        )))
        .expect("DVC compressed data should encode")
    };
    // An RDP 8.0 Lite segment carried uncompressed, which still enters the history.
    let literal = [0xe0, 0x06, b'x', b'y', b'z'];
    // A compressed segment holding one match: distance 3, length 3.
    let back_reference = [0xe0, 0x26, 0b1000_1000, 0b1100_0000, 0x05];

    client
        .process(&data_compressed(3, &literal))
        .expect("DVC compressed data should be processed");
    client
        .process(&data_compressed(3, &back_reference))
        .expect("DVC compressed data should be processed");
    client
        .process(&data_compressed(4, &back_reference))
        .expect("DVC compressed data should be processed");

    let received = |channel_id| {
        client
            .get_dvc_by_channel_id::<RecordedDvc>(channel_id)
            .expect("recorded channel should remain attached")
            .processor()
            .received
            .clone()
    };
    assert_eq!(received(3), vec![b"xyz".to_vec(), b"xyz".to_vec()]);
    // Channel 4 never saw "xyz", so the match reads its still-empty history.
    assert_eq!(received(4), vec![vec![0; 3]]);
}

#[test]
fn compressed_first_block_longer_than_the_message_is_an_error() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(3, RecordedDvc::default())
        .expect("recorded channel should attach");

    // The MS-RDPEDYC 4.3.3 block decompresses to 1,595 bytes, more than the 100 declared.
    let pdu = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::DataFirstCompressed(
        DataFirstCompressedPdu::new(3, 100, vec![0xe0, 0x26, 0x38, 0xc4, 0x3f, 0xf4, 0x74, 0x01]),
    )))
    .expect("DVC compressed data should encode");

    assert!(client.process(&pdu).is_err());
}

#[test]
fn blocks_longer_than_the_message_are_an_error() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(3, RecordedDvc::default())
        .expect("recorded channel should attach");

    let data_first = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::DataFirst(DataFirstPdu::new(
        3,
        4,
        b"ab".to_vec(),
    ))))
    .expect("DVC data should encode");
    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        3,
        b"cde".to_vec(),
    ))))
    .expect("DVC data should encode");

    assert!(
        client
            .process(&data_first)
            .expect("first block should be buffered")
            .is_empty()
    );
    assert!(client.process(&data).is_err());

    let channel = client
        .get_dvc_by_channel_id::<RecordedDvc>(3)
        .expect("recorded channel should remain attached");
    assert!(channel.processor().received.is_empty());
}

#[test]
fn malformed_compressed_data_is_an_error() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(3, RecordedDvc::default())
        .expect("recorded channel should attach");

    let data_compressed = |data: Vec<u8>| {
        encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::DataCompressed(
            DataCompressedPdu::new(3, data),
        )))
        .expect("DVC compressed data should encode")
    };

    // RDP_SEGMENTED_DATA with an unknown descriptor.
    assert!(client.process(&data_compressed(vec![0x12, 0x26, 0x00])).is_err());

    // The history may be out of sync now, so even a valid compressed block is refused.
    assert!(client.process(&data_compressed(vec![0xe0, 0x06, b'x'])).is_err());

    // Uncompressed data does not use the history and still gets through.
    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        3,
        b"plain".to_vec(),
    ))))
    .expect("DVC data should encode");
    client.process(&data).expect("uncompressed data should be processed");

    let channel = client
        .get_dvc_by_channel_id::<RecordedDvc>(3)
        .expect("recorded channel should remain attached");
    assert_eq!(channel.processor().received, vec![b"plain".to_vec()]);
}

#[test]
fn soft_sync_rejects_a_tunnel_that_became_unavailable_before_migration() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");
    client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);
    client.disable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);

    let request = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![7]),
    ])))
    .expect("Soft-Sync request should encode");

    // MS-RDPEDYC 3.2.5.3.1: the server manager starts sending data for these channels on the
    // tunnel as soon as it sends the request, so a tunnel we can no longer honor must fail the
    // whole request rather than being silently dropped from the response.
    assert!(client.process(&request).is_err());
    assert!(!client.soft_sync_complete());
    assert_eq!(client.tunnel_for_channel(7), None);
    assert!(!client.has_channels_on_tunnel(SoftSyncTunnelType::RELIABLE_UDP));
}

#[test]
fn soft_sync_skips_channels_the_client_did_not_open() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");
    client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);

    // Windows lists the channels this client answered with NO_LISTENER (2, 6, 8..=12) next to
    // the graphics pipeline it accepted (7). The tunnel must still be switched for channel 7.
    let request = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![2, 6, 7, 8, 9, 10, 11, 12]),
    ])))
    .expect("Soft-Sync request should encode");

    let responses = client.process(&request).expect("Soft-Sync request should be accepted");
    let [response] = responses.as_slice() else {
        panic!("expected exactly one Soft-Sync response");
    };
    let encoded = response
        .encode_unframed_pdu()
        .expect("Soft-Sync response should encode");
    let DrdynvcClientPdu::SoftSyncResponse(response) =
        DrdynvcClientPdu::decode(&mut ReadCursor::new(&encoded)).expect("Soft-Sync response should decode")
    else {
        panic!("expected a Soft-Sync response");
    };
    assert_eq!(response.tunnels_to_switch(), &[SoftSyncTunnelType::RELIABLE_UDP]);

    assert!(client.soft_sync_complete());
    assert_eq!(client.tunnel_for_channel(7), Some(SoftSyncTunnelType::RELIABLE_UDP));
    assert_eq!(client.tunnel_for_channel(2), None);
    assert!(client.has_channels_on_tunnel(SoftSyncTunnelType::RELIABLE_UDP));
}

#[test]
fn message_batch_rejects_a_mismatched_channel_id() {
    let message = SvcMessage::from(DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        7,
        Vec::new(),
    ))));

    assert!(DvcMessageBatch::try_new(8, vec![message]).is_err());
}

#[test]
fn channels_created_on_a_tunnel_are_bound_to_it() {
    let mut client = DrdynvcClient::new().with_dynamic_channel(TunnelCreatedDvc);
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");
    client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);

    let create = encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
        16,
        "tunnel-created".to_owned(),
    )))
    .expect("Create Request should encode");
    // Nothing may arrive on a tunnel before Soft-Sync completes.
    assert!(
        client
            .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &create)
            .is_err()
    );

    let soft_sync = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![7]),
    ])))
    .expect("Soft-Sync request should encode");
    client
        .process(&soft_sync)
        .expect("Soft-Sync request should be accepted");

    // Windows opens channels such as AUDIO_PLAYBACK_DVC directly on the tunnel after Soft-Sync.
    let batch = client
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &create)
        .expect("Create Request on the tunnel should be processed");
    assert_eq!(batch.channel_id(), 16);
    let [response] = batch.messages() else {
        panic!("expected exactly one Create Response");
    };
    let DrdynvcClientPdu::Create(response) = decode_client_pdu(response) else {
        panic!("expected a Create Response");
    };
    assert_eq!(response.creation_status(), CreationStatus::OK);
    assert_eq!(client.tunnel_for_channel(16), Some(SoftSyncTunnelType::RELIABLE_UDP));

    // Data for the new channel flows on the tunnel; the same data over TCP is rejected.
    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        16,
        Vec::new(),
    ))))
    .expect("DVC data should encode");
    assert!(client.process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &data).is_ok());
    assert!(client.process(&data).is_err());

    // A channel without a listener is declined on the tunnel and not bound to it.
    let unknown = encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
        17,
        "unknown".to_owned(),
    )))
    .expect("Create Request should encode");
    let batch = client
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &unknown)
        .expect("declined Create Request should still be answered");
    assert_eq!(batch.channel_id(), 17);
    let [response] = batch.messages() else {
        panic!("expected exactly one Create Response");
    };
    let DrdynvcClientPdu::Create(response) = decode_client_pdu(response) else {
        panic!("expected a Create Response");
    };
    assert_eq!(response.creation_status(), CreationStatus::NO_LISTENER);
    assert_eq!(client.tunnel_for_channel(17), None);

    // Closing the channel on the tunnel answers the Close and unbinds it.
    let close = encode_vec(&DrdynvcServerPdu::Close(ClosePdu::new(16))).expect("Close should encode");
    let batch = client
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &close)
        .expect("Close on the tunnel should be processed");
    let [response] = batch.messages() else {
        panic!("expected exactly one Close response");
    };
    assert!(matches!(decode_client_pdu(response), DrdynvcClientPdu::Close(_)));
    assert_eq!(client.tunnel_for_channel(16), None);
}

#[test]
fn tunnel_stays_in_use_after_its_channels_close() {
    let mut client = DrdynvcClient::new().with_dynamic_channel(TunnelCreatedDvc);
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");
    client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);

    let soft_sync = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![7]),
    ])))
    .expect("Soft-Sync request should encode");
    client
        .process(&soft_sync)
        .expect("Soft-Sync request should be accepted");
    assert!(client.switched_to_tunnel(SoftSyncTunnelType::RELIABLE_UDP));

    let close = encode_vec(&DrdynvcServerPdu::Close(ClosePdu::new(7))).expect("Close should encode");
    client.process(&close).expect("Close should be processed");
    assert!(!client.has_channels_on_tunnel(SoftSyncTunnelType::RELIABLE_UDP));
    assert!(client.switched_to_tunnel(SoftSyncTunnelType::RELIABLE_UDP));

    // The server can still open a channel on the tunnel after the last one closed.
    let create = encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
        16,
        "tunnel-created".to_owned(),
    )))
    .expect("Create Request should encode");
    client
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &create)
        .expect("Create Request on the tunnel should be processed");
    assert_eq!(client.tunnel_for_channel(16), Some(SoftSyncTunnelType::RELIABLE_UDP));
}

#[test]
fn tunnel_refuses_a_create_request_before_the_capabilities_exchange() {
    let mut client = DrdynvcClient::new().with_dynamic_channel(TunnelCreatedDvc);
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");
    client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);

    let soft_sync = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![7]),
    ])))
    .expect("Soft-Sync request should encode");
    client
        .process(&soft_sync)
        .expect("Soft-Sync request should be accepted");

    let create = encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
        16,
        "tunnel-created".to_owned(),
    )))
    .expect("Create Request should encode");
    assert!(
        client
            .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &create)
            .is_err()
    );
    assert_eq!(client.tunnel_for_channel(16), None);

    exchange_capabilities(&mut client);
    client
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &create)
        .expect("Create Request on the tunnel should be processed after the capabilities exchange");
    assert_eq!(client.tunnel_for_channel(16), Some(SoftSyncTunnelType::RELIABLE_UDP));
}

// oracle: spec MS-RDPEDYC section 3.2.5.2 - ignore a close for an already closed channel.
#[test]
fn data_and_server_close_racing_a_client_close_are_dropped() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");

    let close = client.close_channel(7).expect("open channel should close");
    assert!(matches!(decode_client_pdu(&close), DrdynvcClientPdu::Close(_)));

    // The server sent this before it received the Close PDU.
    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        7,
        b"late".to_vec(),
    ))))
    .expect("DVC data should encode");
    assert!(
        client
            .process(&data)
            .expect("late data for a closing channel should be dropped")
            .is_empty()
    );

    // A server Close sent before receiving the client's Close needs no response.
    let server_close = encode_vec(&DrdynvcServerPdu::Close(ClosePdu::new(7))).expect("Close should encode");
    assert!(
        client
            .process(&server_close)
            .expect("racing Close should be processed")
            .is_empty()
    );
    client
        .process(&data)
        .expect_err("data for a channel that is fully closed is still an error");
}

#[test]
fn client_close_of_a_tunneled_channel_reports_its_tunnel() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("tunneled channel should attach");
    client
        .attach_established_dynamic_channel(9, RecordedDvc::default())
        .expect("TCP channel should attach");
    client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);
    let soft_sync = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![7]),
    ])))
    .expect("Soft-Sync request should encode");
    client
        .process(&soft_sync)
        .expect("Soft-Sync request should be accepted");

    let (batch, tunnel) = client.close_channel_batch(7).expect("tunneled channel should close");
    assert_eq!(tunnel, Some(SoftSyncTunnelType::RELIABLE_UDP));
    assert_eq!(batch.channel_id(), 7);
    assert!(matches!(
        decode_client_pdu(&batch.messages()[0]),
        DrdynvcClientPdu::Close(_)
    ));

    // Data the server sent on the tunnel before it received the Close is dropped.
    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        7,
        b"late".to_vec(),
    ))))
    .expect("DVC data should encode");
    let late = client
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &data)
        .expect("late tunneled data for a closing channel should be dropped");
    assert!(late.messages().is_empty());

    let (_, tunnel) = client.close_channel_batch(9).expect("TCP channel should close");
    assert_eq!(tunnel, None);
    assert!(client.close_channel_batch(9).is_none());
}

// oracle: regression #2181 - channel IDs need not increase with the order of client closes.
#[test]
fn client_closed_channels_are_forgotten_in_close_order() {
    let mut client = DrdynvcClient::new();
    exchange_capabilities(&mut client);
    for channel_id in (1..=100).rev() {
        client
            .attach_established_dynamic_channel(channel_id, RecordedDvc::default())
            .expect("channel should attach");
        client.close_channel(channel_id).expect("open channel should close");
    }

    let data = |channel_id| {
        encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
            channel_id,
            b"late".to_vec(),
        ))))
        .expect("DVC data should encode")
    };

    // Recent closes still absorb late data; the oldest are forgotten, which bounds the bookkeeping.
    assert!(client.process(&data(1)).expect("late data is dropped").is_empty());
    client
        .process(&data(100))
        .expect_err("a long-forgotten close no longer absorbs data");
}

// oracle: regression #2181 - restoring a closed channel must route data to its new processor.
#[test]
fn restored_channel_reuses_a_client_closed_id() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("original channel should attach");
    client.close_channel(7).expect("original channel should close");
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("replacement channel should attach");

    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        7,
        b"replacement data".to_vec(),
    ))))
    .expect("DVC data should encode");
    client.process(&data).expect("replacement channel should receive data");

    let channel = client
        .get_dvc_by_channel_id::<RecordedDvc>(7)
        .expect("replacement channel should remain attached");
    assert_eq!(channel.processor().received, vec![b"replacement data".to_vec()]);
}

#[test]
fn tunneled_data_for_a_channel_closed_before_soft_sync_is_dropped() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");
    client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);

    // The client closes the channel while the server is already choosing tunnels, and lists it.
    client.close_channel(7).expect("open channel should close");
    let request = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![7]),
    ])))
    .expect("Soft-Sync request should encode");
    client.process(&request).expect("Soft-Sync request should be accepted");
    assert_eq!(client.tunnel_for_channel(7), None);

    // Data the server sent on the tunnel before it saw the close is dropped.
    let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        7,
        vec![1, 2, 3],
    ))))
    .expect("data should encode");
    let batch = client
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &data)
        .expect("late tunneled data is dropped");
    assert!(batch.messages().is_empty());
}
