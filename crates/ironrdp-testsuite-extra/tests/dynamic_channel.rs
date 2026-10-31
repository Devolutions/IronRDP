use core::any::TypeId;
use core::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use ironrdp_core::{ReadCursor, decode, decode_cursor, encode_vec, impl_as_any};
use ironrdp_dvc::pdu::{
    CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateResponsePdu, CreationStatus, DrdynvcClientPdu,
    DrdynvcServerPdu,
};
use ironrdp_dvc::{DrdynvcServer, DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::mcs::SendDataIndication;
use ironrdp_pdu::rdp::vc::ChannelPduHeader;
use ironrdp_pdu::x224::X224;
use ironrdp_server::{RdpServer, ServerEvent, SharedWriter, dispatch_static_channel_events};
use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};
use ironrdp_tokio::TokioFramed;
use rstest::rstest;
use tokio::io::AsyncReadExt as _;

const CHANNEL_ID: u16 = 1005;

#[derive(Debug, PartialEq, Eq)]
enum Call {
    Built(u32),
    Started(u32),
    Closed(u32),
    Dropped,
}

type Calls = Arc<Mutex<Vec<Call>>>;

struct Processor(Calls);

impl_as_any!(Processor);

impl DvcProcessor for Processor {
    fn channel_name(&self) -> &str {
        "runtime-test"
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.0.lock().unwrap().push(Call::Started(channel_id));
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn close(&mut self, channel_id: u32) {
        self.0.lock().unwrap().push(Call::Closed(channel_id));
    }
}

impl DvcServerProcessor for Processor {}

impl Drop for Processor {
    fn drop(&mut self) {
        self.0.lock().unwrap().push(Call::Dropped);
    }
}

fn server() -> RdpServer {
    RdpServer::builder()
        .with_addr((Ipv4Addr::LOCALHOST, 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build()
}

fn channels(joined: bool, ready: bool) -> StaticChannelSet {
    let mut drdynvc = DrdynvcServer::new().with_dynamic_channel(Processor(Calls::default()));
    drdynvc.start().unwrap();
    if ready {
        drdynvc
            .process(
                &encode_vec(&DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(
                    CapsVersion::V3,
                )))
                .unwrap(),
            )
            .unwrap();
    }
    let mut channels = StaticChannelSet::new();
    channels.insert(drdynvc);
    if joined {
        channels.attach_channel_id(TypeId::of::<DrdynvcServer>(), CHANNEL_ID);
    }
    channels
}

fn create(calls: &Calls) -> ServerEvent {
    let processor = Processor(Arc::clone(calls));
    ServerEvent::CreateDynamicChannel(Box::new(move |id| {
        processor.0.lock().unwrap().push(Call::Built(id));
        Box::new(processor)
    }))
}

fn peer(channels: &mut StaticChannelSet, pdu: DrdynvcClientPdu) {
    channels
        .get_by_type_mut::<DrdynvcServer>()
        .unwrap()
        .channel_processor_downcast_mut::<DrdynvcServer>()
        .unwrap()
        .process(&encode_vec(&pdu).unwrap())
        .unwrap();
}

async fn dispatch(
    server: &mut RdpServer,
    channels: StaticChannelSet,
    mut events: Vec<ServerEvent>,
) -> (StaticChannelSet, Vec<DrdynvcServerPdu>) {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let mut transport = TokioFramed::new(stream);
    let channels =
        dispatch_static_channel_events(server, channels, &mut events, &mut SharedWriter::new(&mut transport))
            .await
            .unwrap();
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    let mut frames = ReadCursor::new(&bytes);
    let mut pdus = Vec::new();
    while !frames.is_empty() {
        let frame: X224<SendDataIndication<'_>> = decode_cursor(&mut frames).unwrap();
        assert_eq!(frame.0.channel_id, CHANNEL_ID);
        assert_eq!(frame.0.initiator_id, 1002);
        let mut chunk = ReadCursor::new(frame.0.user_data.as_ref());
        let _: ChannelPduHeader = decode_cursor(&mut chunk).unwrap();
        pdus.push(decode(chunk.remaining()).unwrap());
    }
    (channels, pdus)
}

async fn requested(server: &mut RdpServer, calls: &Calls) -> (StaticChannelSet, u32) {
    let registered = channels(true, true);
    let existing_id = registered
        .get_by_type::<DrdynvcServer>()
        .unwrap()
        .channel_processor_downcast_ref::<DrdynvcServer>()
        .unwrap()
        .get_channel_id_by_type::<Processor>()
        .unwrap();
    let (channels, pdus) = dispatch(server, registered, vec![create(calls)]).await;
    assert_eq!(pdus.len(), 1);
    let request = pdus
        .into_iter()
        .find_map(|pdu| match pdu {
            DrdynvcServerPdu::Create(request) => Some(request),
            _ => None,
        })
        .expect("expected a Create PDU");
    assert_eq!(request.channel_name(), "runtime-test");
    let id = request.channel_id();
    assert_ne!(id, existing_id);
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id)]);
    (channels, id)
}

// oracle: spec MS-RDPEDYC §3.3.3, "Initialization"; mutant invoking a builder before DRDYNVC is usable.
#[rstest]
#[case::unjoined(false, true)]
#[case::negotiating(true, false)]
#[tokio::test]
async fn creation_requires_joined_and_ready_drdynvc(#[case] joined: bool, #[case] ready: bool) {
    let calls = Calls::default();
    let (_, pdus) = dispatch(&mut server(), channels(joined, ready), vec![create(&calls)]).await;
    assert!(pdus.is_empty());
    assert_eq!(*calls.lock().unwrap(), [Call::Dropped]);
}

// oracle: spec MS-RDPEDYC §3.3.3, "Initialization"; mutant building a processor without DRDYNVC.
#[tokio::test]
async fn missing_drdynvc_discards_creation() {
    let calls = Calls::default();
    let (_, pdus) = dispatch(&mut server(), StaticChannelSet::new(), vec![create(&calls)]).await;
    assert!(pdus.is_empty());
    assert_eq!(*calls.lock().unwrap(), [Call::Dropped]);
}

// oracle: mutant emitting Close for an unknown channel or treating absent DRDYNVC as a connection failure.
#[rstest]
#[case::missing(StaticChannelSet::new())]
#[case::unjoined(channels(false, false))]
#[case::unknown(channels(true, true))]
#[tokio::test]
async fn unavailable_channel_close_is_a_no_op(#[case] channels: StaticChannelSet) {
    let (_, pdus) = dispatch(
        &mut server(),
        channels,
        vec![ServerEvent::CloseDynamicChannel(u32::MAX)],
    )
    .await;
    assert!(pdus.is_empty());
}

// oracle: spec MS-RDPEDYC §3.3.3.2 and §3.3.5.2; mutant starting before acceptance or closing twice.
#[tokio::test]
async fn acceptance_starts_the_reserved_processor_and_close_releases_it_once() {
    let calls = Calls::default();
    let mut server = server();
    let (mut channels, id) = requested(&mut server, &calls).await;
    peer(
        &mut channels,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::OK)),
    );
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Started(id)]);
    let (channels, pdus) = dispatch(
        &mut server,
        channels,
        vec![
            ServerEvent::CloseDynamicChannel(id),
            ServerEvent::CloseDynamicChannel(id),
            ServerEvent::CloseDynamicChannel(u32::MAX),
        ],
    )
    .await;
    assert_eq!(pdus, [DrdynvcServerPdu::Close(ClosePdu::new(id))]);
    assert_eq!(
        *calls.lock().unwrap(),
        [Call::Built(id), Call::Started(id), Call::Closed(id), Call::Dropped]
    );
    drop(channels);
    assert_eq!(
        *calls.lock().unwrap(),
        [Call::Built(id), Call::Started(id), Call::Closed(id), Call::Dropped]
    );
}

// oracle: mutant starting a canceled processor on late acceptance or reusing its ID.
#[tokio::test]
async fn close_cancels_pending_creation_and_late_acceptance_cannot_start_it() {
    let calls = Calls::default();
    let mut server = server();
    let (channels, id) = requested(&mut server, &calls).await;
    let (mut channels, pdus) = dispatch(&mut server, channels, vec![ServerEvent::CloseDynamicChannel(id)]).await;
    assert_eq!(pdus, [DrdynvcServerPdu::Close(ClosePdu::new(id))]);
    peer(
        &mut channels,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::OK)),
    );
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Dropped]);

    let replacement = Calls::default();
    let (channels, pdus) = dispatch(&mut server, channels, vec![create(&replacement)]).await;
    let [DrdynvcServerPdu::Create(request)] = pdus.as_slice() else {
        panic!("expected a replacement Create PDU");
    };
    assert_ne!(request.channel_id(), id);
    assert_eq!(*replacement.lock().unwrap(), [Call::Built(request.channel_id())]);
    drop(channels);
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Dropped]);
}

// oracle: mutant starting a declined processor or emitting Close for failed creation.
#[tokio::test]
async fn close_releases_a_declined_processor_without_starting_it() {
    let calls = Calls::default();
    let mut server = server();
    let (mut channels, id) = requested(&mut server, &calls).await;
    peer(
        &mut channels,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::NO_LISTENER)),
    );
    let (channels, pdus) = dispatch(&mut server, channels, vec![ServerEvent::CloseDynamicChannel(id)]).await;
    assert!(pdus.is_empty());
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Dropped]);
    drop(channels);
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Dropped]);
}

// oracle: mutant retaining pending creation after connection teardown.
#[tokio::test]
async fn teardown_drops_pending_creation_without_start_or_close() {
    let calls = Calls::default();
    let (channels, id) = requested(&mut server(), &calls).await;
    drop(channels);
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Dropped]);
}

// oracle: mutant omitting or duplicating the opened processor's close callback on teardown.
#[tokio::test]
async fn teardown_closes_an_opened_processor_once() {
    let calls = Calls::default();
    let (mut channels, id) = requested(&mut server(), &calls).await;
    peer(
        &mut channels,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::OK)),
    );
    drop(channels);
    assert_eq!(
        *calls.lock().unwrap(),
        [Call::Built(id), Call::Started(id), Call::Closed(id), Call::Dropped]
    );
}

// oracle: mutant retaining a queued creation across the connection boundary.
#[tokio::test]
async fn stale_creation_is_discarded_without_building_a_processor() {
    let calls = Calls::default();
    let mut server = server();
    server.event_sender().send(create(&calls)).unwrap();
    server.discard_stale_session_events().await;
    assert_eq!(*calls.lock().unwrap(), [Call::Dropped]);
}
