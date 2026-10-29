use core::net::Ipv4Addr;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::Poll;
use core::time::Duration;
use std::sync::Arc;

use ironrdp_async::FramedWrite as _;
use ironrdp_core::{ReadCursor, decode_cursor, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::mcs::SendDataIndication;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_pdu::x224::X224;
use ironrdp_server::{RdpServer, ServerEvent, SharedWriter, dispatch_static_channel_events};
use ironrdp_svc::{StaticChannelSet, SvcMessage, SvcProcessor, SvcServerProcessor};
use ironrdp_tokio::TokioFramed;
use rstest::rstest;
use tokio::io::AsyncReadExt as _;
use tokio::sync::{Semaphore, oneshot};

const NAME: ChannelName = ChannelName::from_static(b"testchn\0");
const CHANNEL_ID: u16 = 1005;

#[derive(Debug)]
struct NamedChannel;

impl_as_any!(NamedChannel);

impl SvcProcessor for NamedChannel {
    fn channel_name(&self) -> ChannelName {
        NAME
    }

    fn process(&mut self, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

impl SvcServerProcessor for NamedChannel {}

fn server() -> RdpServer {
    RdpServer::builder()
        .with_addr((Ipv4Addr::LOCALHOST, 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build()
}

fn channels(joined: bool) -> StaticChannelSet {
    let mut channels = StaticChannelSet::new();
    let key = channels.insert_dynamic(NamedChannel).unwrap();
    if joined {
        channels.attach_channel_id_by_key(key, CHANNEL_ID);
    }
    channels
}

fn write(payload: Vec<u8>, permits: &Arc<Semaphore>) -> (ServerEvent, oneshot::Receiver<()>) {
    let (completion, processed) = oneshot::channel();
    (
        ServerEvent::StaticChannelWithCompletion {
            name: NAME,
            payload,
            completion,
            permit: Arc::clone(permits).try_acquire_owned().unwrap(),
            is_current: Box::new(|| true),
        },
        processed,
    )
}

fn chunks(bytes: &[u8]) -> Vec<(ChannelPduHeader, Vec<u8>)> {
    let mut frames = ReadCursor::new(bytes);
    let mut chunks = Vec::new();
    while !frames.is_empty() {
        let frame: X224<SendDataIndication<'_>> = decode_cursor(&mut frames).unwrap();
        assert_eq!(frame.0.channel_id, CHANNEL_ID);
        assert_eq!(frame.0.initiator_id, 1002);
        let mut chunk = ReadCursor::new(frame.0.user_data.as_ref());
        let header = decode_cursor(&mut chunk).unwrap();
        chunks.push((header, chunk.remaining().to_vec()));
    }
    chunks
}

// oracle: spec MS-RDPBCGR 2.2.6.1.1; one chunk carries the total length and FIRST | LAST.
#[tokio::test]
async fn writes_to_the_named_joined_channel() {
    let permits = Arc::new(Semaphore::new(1));
    let (event, processed) = write(b"static channel data".to_vec(), &permits);
    let (stream, mut peer) = tokio::io::duplex(128);
    let mut transport = TokioFramed::new(stream);
    dispatch_static_channel_events(
        &mut server(),
        channels(true),
        &mut vec![event],
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(
        chunks(&bytes),
        [(
            ChannelPduHeader {
                length: 19,
                flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
            },
            b"static channel data".to_vec(),
        )]
    );
    processed.await.unwrap();
    assert_eq!(permits.available_permits(), 1);
}

async fn assert_pending(future: Pin<&mut impl Future>) {
    let mut future = future;
    core::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

async fn behind_writer(event: ServerEvent, after_wait: impl FnOnce(), permits: &Semaphore) -> Vec<u8> {
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
        let dispatch = dispatch_static_channel_events(&mut server, channels(true), &mut events, &mut writer);
        tokio::pin!(dispatch);
        assert_pending(dispatch.as_mut()).await;
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

// oracle: mutant checking receiver cancellation only before waiting for the writer lock.
#[tokio::test]
async fn cancellation_is_checked_after_the_writer_lock() {
    let permits = Arc::new(Semaphore::new(1));
    let (event, processed) = write(b"canceled behind another writer".to_vec(), &permits);
    let bytes = behind_writer(event, || drop(processed), &permits).await;
    assert!(bytes.is_empty(), "canceled data escaped after the writer lock");
    assert_eq!(permits.available_permits(), 1);
}

// oracle: mutant checking invalidation only before waiting for the writer lock.
#[tokio::test]
async fn invalidation_is_checked_after_the_writer_lock() {
    let permits = Arc::new(Semaphore::new(1));
    let current = Arc::new(AtomicBool::new(true));
    let (completion, processed) = oneshot::channel();
    let event = ServerEvent::StaticChannelWithCompletion {
        name: NAME,
        payload: b"replaced consumer".to_vec(),
        completion,
        permit: Arc::clone(&permits).try_acquire_owned().unwrap(),
        is_current: Box::new({
            let current = Arc::clone(&current);
            move || current.load(Ordering::SeqCst)
        }),
    };
    let bytes = behind_writer(event, || current.store(false, Ordering::SeqCst), &permits).await;
    assert!(bytes.is_empty(), "invalidated data escaped after the writer lock");
    assert!(processed.await.is_err());
    assert_eq!(permits.available_permits(), 1);
}

// oracle: mutant confusing channel registration or another channel's join with the requested channel's join.
#[rstest]
#[case::joined(true, NAME, true)]
#[case::registered_only(false, NAME, false)]
#[case::different_name(true, ChannelName::from_static(b"otherch\0"), false)]
#[tokio::test]
async fn query_reports_the_named_channel_join(
    #[case] joined: bool,
    #[case] queried: ChannelName,
    #[case] expected: bool,
) {
    let (reply, answer) = oneshot::channel();
    let (stream, mut peer) = tokio::io::duplex(128);
    let mut transport = TokioFramed::new(stream);
    dispatch_static_channel_events(
        &mut server(),
        channels(joined),
        &mut vec![ServerEvent::StaticChannelJoined(queried, reply)],
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    assert_eq!(answer.await.unwrap(), expected);
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty(), "a join query wrote to the transport");
}

// oracle: spec MS-RDPBCGR 3.1.5.2.1; chunks are at most 1600 bytes, ordered, with FIRST and LAST on the ends.
#[tokio::test]
async fn fragmented_pdus_preserve_payload_and_queue_order() {
    let permits = Arc::new(Semaphore::new(2));
    let payload: Vec<u8> = (0..3201u32).map(|i| u8::try_from(i % 251).unwrap()).collect();
    let (first, first_done) = write(payload.clone(), &permits);
    let (second, second_done) = write(b"next".to_vec(), &permits);
    let (stream, mut peer) = tokio::io::duplex(8192);
    let mut transport = TokioFramed::new(stream);
    dispatch_static_channel_events(
        &mut server(),
        channels(true),
        &mut vec![first, second],
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    first_done.await.unwrap();
    second_done.await.unwrap();
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    let chunks = chunks(&bytes);
    let boundary = ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST;
    let shape: Vec<_> = chunks
        .iter()
        .map(|(header, data)| (header.length, header.flags & boundary, data.len()))
        .collect();
    assert_eq!(
        shape,
        [
            (3201, ChannelControlFlags::FLAG_FIRST, 1600),
            (3201, ChannelControlFlags::empty(), 1600),
            (3201, ChannelControlFlags::FLAG_LAST, 1),
            (4, boundary, 4),
        ]
    );
    let reassembled: Vec<u8> = chunks[..3].iter().flat_map(|(_, data)| data.iter().copied()).collect();
    assert_eq!(reassembled, payload);
    assert_eq!(chunks[3].1, b"next");
    assert_eq!(permits.available_permits(), 2);
}

async fn read_frame(peer: &mut tokio::io::DuplexStream) -> Vec<u8> {
    let mut header = [0; 4];
    peer.read_exact(&mut header).await.unwrap();
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    assert!((4..=8192).contains(&length), "invalid TPKT length");
    let mut bytes = vec![0; length];
    bytes[..4].copy_from_slice(&header);
    peer.read_exact(&mut bytes[4..]).await.unwrap();
    bytes
}

// oracle: mutant acknowledging a stalled write, releasing its permit, or allowing another writer between its chunks.
#[tokio::test]
async fn completion_and_permit_wait_for_all_chunks_without_interleaving() {
    let permits = Arc::new(Semaphore::new(1));
    let payload = vec![0x42; 3201];
    let (event, mut processed) = write(payload.clone(), &permits);
    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut writer = SharedWriter::new(&mut transport);
    let mut competing_writer = writer.clone();
    let mut server = server();
    let mut events = vec![event];
    let dispatch = dispatch_static_channel_events(&mut server, channels(true), &mut events, &mut writer);
    tokio::pin!(dispatch);
    assert_pending(dispatch.as_mut()).await;
    assert_eq!(processed.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    assert!(Arc::clone(&permits).try_acquire_owned().is_err());
    let competing = competing_writer.write_all(b"next");
    tokio::pin!(competing);
    assert_pending(competing.as_mut()).await;

    let bytes = tokio::time::timeout(Duration::from_secs(5), async {
        let (sent, next, bytes) = tokio::join!(&mut dispatch, &mut competing, async {
            let mut bytes = Vec::new();
            for _ in 0..3 {
                bytes.extend(read_frame(&mut peer).await);
            }
            let mut next = [0; 4];
            peer.read_exact(&mut next).await.unwrap();
            assert_eq!(next, *b"next");
            bytes
        });
        sent.unwrap();
        next.unwrap();
        bytes
    })
    .await
    .unwrap();
    assert_eq!(processed.try_recv(), Ok(()));
    let reassembled: Vec<u8> = chunks(&bytes).into_iter().flat_map(|(_, data)| data).collect();
    assert_eq!(reassembled, payload);
    assert_eq!(permits.available_permits(), 1);
}

// oracle: mutant truncating an in-flight RDP frame or releasing its permit when its producer cancels.
#[tokio::test]
async fn cancellation_after_transmission_starts_finishes_the_frame() {
    let permits = Arc::new(Semaphore::new(1));
    let (event, processed) = write(b"finish this frame".to_vec(), &permits);
    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut writer = SharedWriter::new(&mut transport);
    let mut server = server();
    let mut events = vec![event];
    let dispatch = dispatch_static_channel_events(&mut server, channels(true), &mut events, &mut writer);
    tokio::pin!(dispatch);
    assert_pending(dispatch.as_mut()).await;
    drop(processed);
    assert_pending(dispatch.as_mut()).await;
    assert_eq!(permits.available_permits(), 0);

    let bytes = tokio::time::timeout(Duration::from_secs(5), async {
        let (sent, bytes) = tokio::join!(&mut dispatch, read_frame(&mut peer));
        sent.unwrap();
        bytes
    })
    .await
    .unwrap();
    assert_eq!(chunks(&bytes)[0].1, b"finish this frame");
    assert_eq!(permits.available_permits(), 1);
}

// oracle: mutant acknowledging rejected writes or routing them to an unrelated joined channel.
#[rstest]
#[case::unjoined(false, NAME)]
#[case::different_name(true, ChannelName::from_static(b"otherch\0"))]
#[tokio::test]
async fn unjoined_destination_is_rejected(#[case] joined: bool, #[case] name: ChannelName) {
    let permits = Arc::new(Semaphore::new(1));
    let (completion, processed) = oneshot::channel();
    let event = ServerEvent::StaticChannelWithCompletion {
        name,
        payload: b"must not be sent".to_vec(),
        completion,
        permit: Arc::clone(&permits).try_acquire_owned().unwrap(),
        is_current: Box::new(|| true),
    };
    let (stream, mut peer) = tokio::io::duplex(128);
    let mut transport = TokioFramed::new(stream);
    dispatch_static_channel_events(
        &mut server(),
        channels(joined),
        &mut vec![event],
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    assert!(processed.await.is_err());
    assert_eq!(permits.available_permits(), 1);
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty(), "data reached an unjoined destination");
}

// oracle: mutant acknowledging failed transport writes or leaking their producer permits.
#[tokio::test]
async fn transport_failure_closes_completion_and_releases_the_permit() {
    let permits = Arc::new(Semaphore::new(1));
    let (event, processed) = write(b"cannot be delivered".to_vec(), &permits);
    let (stream, peer) = tokio::io::duplex(1);
    drop(peer);
    let mut transport = TokioFramed::new(stream);
    assert!(
        dispatch_static_channel_events(
            &mut server(),
            channels(true),
            &mut vec![event],
            &mut SharedWriter::new(&mut transport),
        )
        .await
        .is_err()
    );
    assert!(processed.await.is_err());
    assert_eq!(permits.available_permits(), 1);
}

// oracle: mutant retaining the previous connection's queued channel data, join queries, or producer permits.
#[tokio::test]
async fn stale_session_events_close_replies_and_release_permits() {
    let mut server = server();
    let permits = Arc::new(Semaphore::new(1));
    let (event, processed) = write(b"previous connection".to_vec(), &permits);
    let (reply, joined) = oneshot::channel();
    server.event_sender().send(event).unwrap();
    server
        .event_sender()
        .send(ServerEvent::StaticChannelJoined(NAME, reply))
        .unwrap();
    server.discard_stale_session_events().await;
    assert!(processed.await.is_err());
    assert!(joined.await.is_err());
    assert_eq!(permits.available_permits(), 1);
}
