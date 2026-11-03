use core::any::TypeId;
use core::net::Ipv4Addr;
use core::sync::atomic::{AtomicBool, Ordering};
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

fn close(channel_id: u32) -> ServerEvent {
    ServerEvent::CloseDynamicChannel {
        channel_id,
        is_current: Box::new(|| true),
    }
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
    let (_, pdus) = dispatch(&mut server(), channels, vec![close(u32::MAX)]).await;
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
    let (channels, pdus) = dispatch(&mut server, channels, vec![close(id), close(id), close(u32::MAX)]).await;
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
    let (mut channels, pdus) = dispatch(&mut server, channels, vec![close(id)]).await;
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
    let (channels, pdus) = dispatch(&mut server, channels, vec![close(id)]).await;
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

// oracle: regression Devolutions/IronRDP#2336; a late close must not affect a reused ID in another connection.
#[tokio::test]
async fn old_connection_close_submitted_after_queue_draining_cannot_close_the_reused_id() {
    let mut server = server();
    let old_calls = Calls::default();
    let current = Arc::new(AtomicBool::new(true));
    let (old_channels, old_id) = requested(&mut server, &old_calls).await;
    drop(old_channels);
    current.store(false, Ordering::SeqCst);
    server.discard_stale_session_events().await;

    let calls = Calls::default();
    let (mut channels, id) = requested(&mut server, &calls).await;
    assert_eq!(
        id, old_id,
        "the new connection must reuse the old ID for this regression"
    );
    peer(
        &mut channels,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(id, CreationStatus::OK)),
    );
    let late_close = ServerEvent::CloseDynamicChannel {
        channel_id: old_id,
        is_current: Box::new(move || current.load(Ordering::SeqCst)),
    };
    let (channels, pdus) = dispatch(&mut server, channels, vec![late_close]).await;
    assert!(pdus.is_empty());
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Started(id)]);
    assert!(
        channels
            .get_by_type::<DrdynvcServer>()
            .unwrap()
            .channel_processor_downcast_ref::<DrdynvcServer>()
            .unwrap()
            .is_channel_opened(id)
    );
    let (channels, pdus) = dispatch(&mut server, channels, vec![close(id)]).await;
    assert_eq!(pdus, [DrdynvcServerPdu::Close(ClosePdu::new(id))]);
    assert_eq!(
        *calls.lock().unwrap(),
        [Call::Built(id), Call::Started(id), Call::Closed(id), Call::Dropped]
    );
    drop(channels);
}

// oracle: regression Devolutions/IronRDP#2336; validity must be checked at dispatch, not event construction.
#[tokio::test]
async fn invalidated_close_preserves_pending_creation_until_a_current_close() {
    let calls = Calls::default();
    let mut server = server();
    let (channels, id) = requested(&mut server, &calls).await;
    let current = Arc::new(AtomicBool::new(true));
    let event = ServerEvent::CloseDynamicChannel {
        channel_id: id,
        is_current: Box::new({
            let current = Arc::clone(&current);
            move || current.load(Ordering::SeqCst)
        }),
    };
    current.store(false, Ordering::SeqCst);
    let (channels, pdus) = dispatch(&mut server, channels, vec![event]).await;
    assert!(pdus.is_empty());
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id)]);
    let (channels, pdus) = dispatch(&mut server, channels, vec![close(id)]).await;
    assert_eq!(pdus, [DrdynvcServerPdu::Close(ClosePdu::new(id))]);
    assert_eq!(*calls.lock().unwrap(), [Call::Built(id), Call::Dropped]);
    drop(channels);
}

mod completion {
    use core::any::TypeId;
    use core::net::Ipv4Addr;
    use core::pin::Pin;
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::task::Poll;
    use core::time::Duration;
    use std::sync::Arc;

    use ironrdp_async::FramedWrite as _;
    use ironrdp_core::{ReadCursor, decode_cursor, encode_vec, impl_as_any};
    use ironrdp_dvc::pdu::{
        CapabilitiesRequestPdu, CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateRequestPdu, CreateResponsePdu,
        CreationStatus, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu, DrdynvcServerPdu, SoftSyncChannelList,
        SoftSyncRequestPdu, SoftSyncTunnelType,
    };
    use ironrdp_dvc::{DrdynvcClient, DrdynvcServer, DvcClientProcessor, DvcMessage, DvcMessageBatch, DvcProcessor};
    use ironrdp_pdu::PduResult;
    use ironrdp_pdu::mcs::SendDataIndication;
    use ironrdp_pdu::rdp::vc::ChannelPduHeader;
    use ironrdp_pdu::x224::X224;
    use ironrdp_server::{EchoDvcBridge, RdpServer, ServerEvent, SharedWriter, dispatch_static_channel_events};
    use ironrdp_svc::{StaticChannelSet, SvcMessage, SvcProcessor as _};
    use ironrdp_tokio::TokioFramed;
    use rstest::rstest;
    use tokio::io::AsyncReadExt as _;
    use tokio::sync::{Semaphore, oneshot};

    const SVC_ID: u16 = 1004;
    const PAYLOAD: &[u8] = b"native channel data";

    fn server() -> RdpServer {
        RdpServer::builder()
            .with_addr((Ipv4Addr::LOCALHOST, 0))
            .with_no_security()
            .with_no_input()
            .with_no_display()
            .build()
    }

    fn registered(server: &RdpServer) -> DrdynvcServer {
        let mut drdynvc = DrdynvcServer::new().with_dynamic_channel(EchoDvcBridge::new(server.echo_handle().clone()));
        drdynvc.start().unwrap();
        drdynvc
            .process(
                &encode_vec(&DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(
                    CapsVersion::V3,
                )))
                .unwrap(),
            )
            .unwrap();
        drdynvc
    }

    fn opened(server: &RdpServer) -> (DrdynvcServer, u32) {
        let mut drdynvc = registered(server);
        let channel_id = drdynvc.get_channel_id_by_type::<EchoDvcBridge>().unwrap();
        drdynvc
            .process(
                &encode_vec(&DrdynvcClientPdu::Create(CreateResponsePdu::new(
                    channel_id,
                    CreationStatus::OK,
                )))
                .unwrap(),
            )
            .unwrap();
        (drdynvc, channel_id)
    }

    fn channels(drdynvc: DrdynvcServer) -> StaticChannelSet {
        let mut channels = StaticChannelSet::new();
        channels.insert(drdynvc);
        channels.attach_channel_id(TypeId::of::<DrdynvcServer>(), SVC_ID);
        channels
    }

    fn write(channel_id: u32) -> (ServerEvent, oneshot::Receiver<()>) {
        let permits = Arc::new(Semaphore::new(1));
        write_if_current(channel_id, &permits, Box::new(|| true))
    }

    fn current_producer() -> (Arc<AtomicBool>, Box<dyn Fn() -> bool + Send>) {
        let current = Arc::new(AtomicBool::new(true));
        let is_current = Box::new({
            let current = Arc::clone(&current);
            move || current.load(Ordering::SeqCst)
        });
        (current, is_current)
    }

    fn write_if_current(
        channel_id: u32,
        permits: &Arc<Semaphore>,
        is_current: Box<dyn Fn() -> bool + Send>,
    ) -> (ServerEvent, oneshot::Receiver<()>) {
        let batch = DvcMessageBatch::try_new(
            channel_id,
            vec![SvcMessage::from(DrdynvcDataPdu::Data(DataPdu::new(
                channel_id,
                PAYLOAD.to_vec(),
            )))],
        )
        .unwrap();
        let (completion, processed) = oneshot::channel();
        (
            ServerEvent::DynamicChannelWithCompletion {
                batch,
                completion,
                permit: Arc::clone(permits).try_acquire_owned().unwrap(),
                is_current,
            },
            processed,
        )
    }

    // oracle: mutant routing an opened DVC batch to another static/dynamic destination or altering its payload.
    #[tokio::test]
    async fn writes_to_the_opened_channel_preserve_payload_and_destination() {
        let mut server = server();
        let (drdynvc, channel_id) = opened(&server);
        let (event, processed) = write(channel_id);
        let (stream, mut peer) = tokio::io::duplex(128);
        let mut transport = TokioFramed::new(stream);
        dispatch_static_channel_events(
            &mut server,
            channels(drdynvc),
            &mut vec![event],
            &mut SharedWriter::new(&mut transport),
        )
        .await
        .unwrap();
        processed.await.unwrap();
        drop(transport);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        assert_payload_and_destination(&bytes, channel_id);
    }

    fn assert_payload_and_destination(bytes: &[u8], channel_id: u32) {
        let mut frames = ReadCursor::new(bytes);
        let frame: X224<SendDataIndication<'_>> = decode_cursor(&mut frames).unwrap();
        assert_eq!(frame.0.channel_id, SVC_ID);
        assert_eq!(frame.0.initiator_id, 1002);
        let mut payload = ReadCursor::new(frame.0.user_data.as_ref());
        let _: ChannelPduHeader = decode_cursor(&mut payload).unwrap();
        let pdu: DrdynvcServerPdu = decode_cursor(&mut payload).unwrap();
        assert_eq!(
            pdu,
            DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(channel_id, PAYLOAD.to_vec())))
        );
        assert!(payload.is_empty());
        assert!(frames.is_empty());
    }

    #[derive(Clone, Copy)]
    enum Unavailable {
        Missing,
        Unjoined,
        Pending,
        Closed,
        Unknown,
        Migrated,
    }

    // oracle: mutant acknowledging or transmitting a batch for an unavailable or sideband-only channel.
    #[rstest]
    #[case::missing(Unavailable::Missing)]
    #[case::unjoined(Unavailable::Unjoined)]
    #[case::pending(Unavailable::Pending)]
    #[case::closed(Unavailable::Closed)]
    #[case::unknown(Unavailable::Unknown)]
    #[case::migrated_before_response(Unavailable::Migrated)]
    #[tokio::test]
    async fn unavailable_destination_is_rejected(#[case] unavailable: Unavailable) {
        let mut server = server();
        let (mut drdynvc, mut channel_id) = opened(&server);
        let channels = match unavailable {
            Unavailable::Missing => StaticChannelSet::new(),
            Unavailable::Unjoined => {
                let mut channels = StaticChannelSet::new();
                channels.insert(drdynvc);
                channels
            }
            Unavailable::Pending => {
                drdynvc = registered(&server);
                channel_id = drdynvc.get_channel_id_by_type::<EchoDvcBridge>().unwrap();
                channels(drdynvc)
            }
            Unavailable::Closed => {
                drdynvc.close_channel(channel_id).unwrap();
                channels(drdynvc)
            }
            Unavailable::Unknown => {
                channel_id = u32::MAX;
                channels(drdynvc)
            }
            Unavailable::Migrated => {
                drdynvc.request_reliable_udp(vec![channel_id]).unwrap();
                channels(drdynvc)
            }
        };
        let (event, processed) = write(channel_id);
        let (stream, mut peer) = tokio::io::duplex(128);
        let mut transport = TokioFramed::new(stream);
        dispatch_static_channel_events(
            &mut server,
            channels,
            &mut vec![event],
            &mut SharedWriter::new(&mut transport),
        )
        .await
        .unwrap();
        assert!(processed.await.is_err(), "an unsent batch was acknowledged");
        drop(transport);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.is_empty(), "rejected data reached the main transport");
    }

    // oracle: mutant completing a DVC event before the entire transport write succeeds or interrupting it after producer invalidation.
    #[tokio::test]
    async fn completion_waits_for_a_stalled_transport_write() {
        let mut server = server();
        let (drdynvc, channel_id) = opened(&server);
        let permits = Arc::new(Semaphore::new(1));
        let (current, is_current) = current_producer();
        let (event, mut processed) = write_if_current(channel_id, &permits, is_current);
        let (stream, mut peer) = tokio::io::duplex(1);
        let mut transport = TokioFramed::new(stream);
        let mut bytes = Vec::new();
        {
            let mut writer = SharedWriter::new(&mut transport);
            let mut events = vec![event];
            let dispatch = dispatch_static_channel_events(&mut server, channels(drdynvc), &mut events, &mut writer);
            tokio::pin!(dispatch);
            assert_pending(dispatch.as_mut()).await;
            assert_eq!(processed.try_recv(), Err(oneshot::error::TryRecvError::Empty));
            // oracle: mutant releasing a permit before its pending write completes.
            assert_eq!(permits.available_permits(), 0);

            // Invalidate only after observing partial transmission, which must still finish.
            let mut first_byte = [0];
            peer.read_exact(&mut first_byte).await.unwrap();
            bytes.extend_from_slice(&first_byte);
            current.store(false, Ordering::SeqCst);
            drain_write(&mut peer, &mut bytes, dispatch.as_mut()).await;
        }
        assert_eq!(processed.try_recv(), Ok(()));
        // oracle: mutant retaining a permit after its write completes.
        assert_eq!(permits.available_permits(), 1);
        drop(transport);
        peer.read_to_end(&mut bytes).await.unwrap();
        assert_payload_and_destination(&bytes, channel_id);
    }

    // oracle: mutant interrupting a partially transmitted DVC PDU when its completion receiver is dropped.
    #[tokio::test]
    async fn cancellation_does_not_interrupt_a_started_transport_write() {
        let mut server = server();
        let (drdynvc, channel_id) = opened(&server);
        let (event, processed) = write(channel_id);
        let (stream, mut peer) = tokio::io::duplex(1);
        let mut transport = TokioFramed::new(stream);
        let mut bytes = Vec::new();
        {
            let mut writer = SharedWriter::new(&mut transport);
            let mut events = vec![event];
            let dispatch = dispatch_static_channel_events(&mut server, channels(drdynvc), &mut events, &mut writer);
            tokio::pin!(dispatch);
            assert_pending(dispatch.as_mut()).await;
            let mut first_byte = [0];
            peer.read_exact(&mut first_byte).await.unwrap();
            bytes.extend_from_slice(&first_byte);
            drop(processed);

            drain_write(&mut peer, &mut bytes, dispatch.as_mut()).await;
        }
        drop(transport);
        peer.read_to_end(&mut bytes).await.unwrap();
        assert_payload_and_destination(&bytes, channel_id);
    }

    async fn drain_write<T>(
        peer: &mut tokio::io::DuplexStream,
        bytes: &mut Vec<u8>,
        mut dispatch: Pin<&mut impl Future<Output = ironrdp_server::ServerResult<T>>>,
    ) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    sent = &mut dispatch => break sent.unwrap(),
                    read = peer.read_buf(bytes) => assert_ne!(read.unwrap(), 0),
                }
            }
        })
        .await
        .unwrap();
    }

    async fn assert_pending(future: Pin<&mut impl Future>) {
        let mut future = future;
        core::future::poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }

    async fn behind_writer(
        permits: &Semaphore,
        event: ServerEvent,
        channels: StaticChannelSet,
        after_wait: impl FnOnce(),
    ) -> Vec<u8> {
        let (stream, mut peer) = tokio::io::duplex(8192);
        let mut transport = TokioFramed::new(stream);
        {
            let mut writer = SharedWriter::new(&mut transport);
            let mut competing_writer = writer.clone();
            let blocker = vec![0; 8193];
            let holding = competing_writer.write_all(&blocker);
            tokio::pin!(holding);
            assert_pending(holding.as_mut()).await;

            let mut server = server();
            let mut events = vec![event];
            let dispatch = dispatch_static_channel_events(&mut server, channels, &mut events, &mut writer);
            tokio::pin!(dispatch);
            assert_pending(dispatch.as_mut()).await;
            // oracle: mutant releasing a permit before writer-lock contention ends.
            assert_eq!(permits.available_permits(), 0);
            after_wait();

            tokio::time::timeout(Duration::from_secs(5), async {
                let mut drained = vec![0; 8193];
                let (written, read) = tokio::join!(&mut holding, peer.read_exact(&mut drained));
                written.unwrap();
                read.unwrap();
                dispatch.await.unwrap();
            })
            .await
            .unwrap();
        }
        drop(transport);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        bytes
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2337#issuecomment-6550401512.
    #[tokio::test]
    async fn stale_producer_cannot_write_to_a_reused_channel_id() {
        let mut server = server();
        let (_, old_channel_id) = opened(&server);
        let (current_drdynvc, current_channel_id) = opened(&server);
        assert_eq!(old_channel_id, current_channel_id);

        let permits = Arc::new(Semaphore::new(1));
        let (current, is_current) = current_producer();
        let (event, processed) = write_if_current(old_channel_id, &permits, is_current);
        current.store(false, Ordering::SeqCst);

        let (stream, mut peer) = tokio::io::duplex(128);
        let mut transport = TokioFramed::new(stream);
        dispatch_static_channel_events(
            &mut server,
            channels(current_drdynvc),
            &mut vec![event],
            &mut SharedWriter::new(&mut transport),
        )
        .await
        .unwrap();
        assert!(processed.await.is_err(), "a stale producer was acknowledged");
        // oracle: mutant retaining a discarded stale event's permit.
        assert_eq!(permits.available_permits(), 1);
        drop(transport);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.is_empty(), "a stale write reached the replacement connection");
    }

    // oracle: mutant checking producer validity only before waiting for the writer lock.
    #[tokio::test]
    async fn producer_invalidation_is_checked_after_the_writer_lock() {
        let server = server();
        let (drdynvc, channel_id) = opened(&server);
        let permits = Arc::new(Semaphore::new(1));
        let (current, is_current) = current_producer();
        let (event, processed) = write_if_current(channel_id, &permits, is_current);
        let bytes = behind_writer(&permits, event, channels(drdynvc), || {
            current.store(false, Ordering::SeqCst)
        })
        .await;
        assert!(bytes.is_empty(), "invalidated data escaped after the writer lock");
        assert!(processed.await.is_err());
        // oracle: mutant retaining an invalidated event's permit.
        assert_eq!(permits.available_permits(), 1);
    }

    // oracle: mutant checking receiver cancellation only before waiting for the writer lock.
    #[tokio::test]
    async fn producer_cancellation_is_checked_after_the_writer_lock() {
        let server = server();
        let (drdynvc, channel_id) = opened(&server);
        let permits = Arc::new(Semaphore::new(1));
        let (event, processed) = write_if_current(channel_id, &permits, Box::new(|| true));
        let bytes = behind_writer(&permits, event, channels(drdynvc), || drop(processed)).await;
        assert!(bytes.is_empty(), "canceled data escaped after the writer lock");
        // oracle: mutant retaining a canceled event's permit.
        assert_eq!(permits.available_permits(), 1);
    }

    // oracle: mutant acknowledging a failed transport write.
    #[tokio::test]
    async fn transport_failure_closes_completion() {
        let mut server = server();
        let (drdynvc, channel_id) = opened(&server);
        let permits = Arc::new(Semaphore::new(1));
        let (event, processed) = write_if_current(channel_id, &permits, Box::new(|| true));
        let (stream, peer) = tokio::io::duplex(1);
        drop(peer);
        let mut transport = TokioFramed::new(stream);
        assert!(
            dispatch_static_channel_events(
                &mut server,
                channels(drdynvc),
                &mut vec![event],
                &mut SharedWriter::new(&mut transport),
            )
            .await
            .is_err()
        );
        assert!(processed.await.is_err());
        // oracle: mutant retaining a failed write's permit.
        assert_eq!(permits.available_permits(), 1);
    }

    // oracle: mutant retaining or acknowledging the previous connection's queued DVC batch.
    #[tokio::test]
    async fn discarded_session_data_closes_completion() {
        let mut server = server();
        let (_, channel_id) = opened(&server);
        let permits = Arc::new(Semaphore::new(1));
        let (event, processed) = write_if_current(channel_id, &permits, Box::new(|| true));
        server.event_sender().send(event).unwrap();
        server.discard_stale_session_events().await;
        assert!(processed.await.is_err());
        // oracle: mutant retaining a discarded session event's permit.
        assert_eq!(permits.available_permits(), 1);
    }

    #[derive(Debug)]
    struct TunnelChannel;

    impl_as_any!(TunnelChannel);

    impl DvcProcessor for TunnelChannel {
        fn channel_name(&self) -> &str {
            "completion-test"
        }

        fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
            Ok(Vec::new())
        }

        fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
            Ok(Vec::new())
        }
    }

    impl DvcClientProcessor for TunnelChannel {}

    fn tunnel_client() -> DrdynvcClient {
        let mut client = DrdynvcClient::new().with_dynamic_channel(TunnelChannel);
        client
            .process(
                &encode_vec(&DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(
                    CapsVersion::V3,
                    None,
                )))
                .unwrap(),
            )
            .unwrap();
        client
    }

    fn tunnel_create(client: &mut DrdynvcClient, channel_id: u32) -> DvcMessageBatch {
        client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);
        client
            .process(
                &encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
                    SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, Vec::new()),
                ])))
                .unwrap(),
            )
            .unwrap();
        let create = encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
            channel_id,
            "completion-test".to_owned(),
        )))
        .unwrap();
        client
            .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &create)
            .unwrap()
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2337#discussion_r4432146507.
    #[tokio::test]
    async fn create_response_batches_are_rejected() {
        let mut server = server();
        let (drdynvc, channel_id) = opened(&server);
        let batch = tunnel_create(&mut tunnel_client(), channel_id);
        assert_control_batch_rejected(&mut server, drdynvc, batch).await;
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2337#discussion_r4432146507.
    #[tokio::test]
    async fn close_response_batches_are_rejected() {
        let mut server = server();
        let (drdynvc, channel_id) = opened(&server);
        let mut client = tunnel_client();
        let _ = tunnel_create(&mut client, channel_id);
        let batch = client
            .process_tunnel(
                SoftSyncTunnelType::RELIABLE_UDP,
                &encode_vec(&DrdynvcServerPdu::Close(ClosePdu::new(channel_id))).unwrap(),
            )
            .unwrap();
        assert_control_batch_rejected(&mut server, drdynvc, batch).await;
    }

    async fn assert_control_batch_rejected(server: &mut RdpServer, drdynvc: DrdynvcServer, batch: DvcMessageBatch) {
        assert!(!batch.messages().is_empty());
        let (completion, processed) = oneshot::channel();
        let permits = Arc::new(Semaphore::new(1));
        let (stream, mut peer) = tokio::io::duplex(128);
        let mut transport = TokioFramed::new(stream);
        dispatch_static_channel_events(
            server,
            channels(drdynvc),
            &mut vec![ServerEvent::DynamicChannelWithCompletion {
                batch,
                completion,
                permit: permits.try_acquire_owned().unwrap(),
                is_current: Box::new(|| true),
            }],
            &mut SharedWriter::new(&mut transport),
        )
        .await
        .unwrap();
        assert!(processed.await.is_err(), "a control response was acknowledged as data");
        drop(transport);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.is_empty(), "a control response reached the peer");
    }
}
