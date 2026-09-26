use ironrdp_core::{Decode as _, ReadCursor, encode_vec, impl_as_any};
use ironrdp_dvc::ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_dvc::pdu::{
    CapabilitiesRequestPdu, CapsVersion, ClosePdu, CreateRequestPdu, CreationStatus, DataPdu, DrdynvcClientPdu,
    DrdynvcDataPdu, DrdynvcServerPdu, SoftSyncChannelList, SoftSyncRequestPdu, SoftSyncTunnelType,
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
fn data_for_a_channel_that_is_not_open_is_dropped() {
    // A server can send data on a channel before it sees the client decline it in the Create
    // Response (GNOME Remote Desktop does this for AUDIO_PLAYBACK_DVC). The session must survive it.
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");
    let data = |channel_id: u32, payload: &[u8]| {
        encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
            channel_id,
            payload.to_vec(),
        ))))
        .expect("DVC data should encode")
    };

    assert!(
        client
            .process(&data(4, b"declined"))
            .expect("data for a channel that is not open should be dropped, not fail")
            .is_empty()
    );
    assert!(
        client
            .process(&data(7, b"open"))
            .expect("open channels are unaffected")
            .is_empty()
    );

    let channel = client
        .get_dvc_by_channel_id::<RecordedDvc>(7)
        .expect("recorded channel should remain attached");
    assert_eq!(channel.processor().received, vec![b"open".to_vec()]);
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
