use core::pin::Pin;
use core::task::Poll;
use core::time::Duration;
use std::any::TypeId;
use std::net::Ipv4Addr;

use ironrdp_cliprdr::CliprdrServer;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId, OwnedFormatDataResponse,
};
use ironrdp_core::impl_as_any;
use ironrdp_server::{RdpServer, ServerEvent, SharedWriter, dispatch_static_channel_events};
use ironrdp_svc::StaticChannelSet;
use ironrdp_tokio::TokioFramed;
use tokio::io::AsyncReadExt as _;
use tokio::sync::oneshot;

const CLIPBOARD_CHANNEL_ID: u16 = 1004;

#[derive(Debug)]
struct SilentBackend;

impl_as_any!(SilentBackend);

impl CliprdrBackend for SilentBackend {
    fn temporary_directory(&self) -> &str {
        "."
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {}

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, _: &[ClipboardFormat]) {}

    fn on_format_data_request(&mut self, _: FormatDataRequest) {}

    fn on_format_data_response(&mut self, _: FormatDataResponse<'_>) {}

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _: LockDataId) {}

    fn on_unlock(&mut self, _: LockDataId) {}
}

fn server() -> RdpServer {
    RdpServer::builder()
        .with_addr((Ipv4Addr::LOCALHOST, 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build()
}

fn clipboard_channels() -> StaticChannelSet {
    let mut channels = StaticChannelSet::new();
    assert!(channels.insert(CliprdrServer::new(Box::new(SilentBackend))).is_none());
    assert!(
        channels
            .attach_channel_id(TypeId::of::<CliprdrServer>(), CLIPBOARD_CHANNEL_ID)
            .is_none()
    );
    channels
}

fn unjoined_clipboard_channel() -> StaticChannelSet {
    let mut channels = StaticChannelSet::new();
    assert!(channels.insert(CliprdrServer::new(Box::new(SilentBackend))).is_none());
    channels
}

fn completion_event(message: ClipboardMessage) -> (ServerEvent, oneshot::Receiver<()>) {
    let (completion, processed) = oneshot::channel();
    (ServerEvent::ClipboardWithCompletion(message, completion), processed)
}

fn copy_message() -> ClipboardMessage {
    ClipboardMessage::SendInitiateCopy(vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
}

async fn assert_pending(future: Pin<&mut impl Future>) {
    let mut future = future;
    core::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

async fn read_frame(peer: &mut tokio::io::DuplexStream) -> Vec<u8> {
    let mut header = [0; 4];
    peer.read_exact(&mut header).await.unwrap();
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    assert!((4..=8192).contains(&length), "invalid TPKT length");
    let mut frame = vec![0; length];
    frame[..4].copy_from_slice(&header);
    peer.read_exact(&mut frame[4..]).await.unwrap();
    frame
}

async fn dispatch_and_capture(events: Vec<ServerEvent>, channels: StaticChannelSet) -> Vec<u8> {
    let (stream, mut peer) = tokio::io::duplex(8192);
    let mut transport = TokioFramed::new(stream);
    let mut server = server();
    let mut events = events;
    dispatch_static_channel_events(
        &mut server,
        channels,
        &mut events,
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    bytes
}

fn frame_count(mut bytes: &[u8]) -> usize {
    let mut frames = 0;
    while !bytes.is_empty() {
        assert!(4 <= bytes.len(), "incomplete TPKT header");
        let length = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
        assert!((4..=bytes.len()).contains(&length), "invalid TPKT length");
        bytes = &bytes[length..];
        frames += 1;
    }
    frames
}

// oracle: mutant resolving completion before the full transport write drains.
#[tokio::test]
async fn completion_waits_for_clipboard_output_to_drain() {
    let (event, mut processed) = completion_event(copy_message());
    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut writer = SharedWriter::new(&mut transport);
    let mut server = server();
    let mut events = vec![event];
    let dispatch = dispatch_static_channel_events(&mut server, clipboard_channels(), &mut events, &mut writer);
    tokio::pin!(dispatch);

    assert_pending(dispatch.as_mut()).await;
    assert_eq!(
        processed.try_recv(),
        Err(oneshot::error::TryRecvError::Empty),
        "completion resolved before the blocked output drained"
    );

    tokio::time::timeout(Duration::from_secs(5), async {
        let (result, frame) = tokio::join!(&mut dispatch, read_frame(&mut peer));
        result.unwrap();
        assert!(!frame.is_empty());
    })
    .await
    .unwrap();
    assert_eq!(processed.try_recv(), Ok(()));
}

// oracle: mutant acknowledging a failed transport write or returning success from dispatch.
#[tokio::test]
async fn transport_failure_closes_completion() {
    let (event, processed) = completion_event(copy_message());
    let (stream, peer) = tokio::io::duplex(1);
    drop(peer);
    let mut transport = TokioFramed::new(stream);
    let mut server = server();
    let mut events = vec![event];

    assert!(
        dispatch_static_channel_events(
            &mut server,
            clipboard_channels(),
            &mut events,
            &mut SharedWriter::new(&mut transport),
        )
        .await
        .is_err()
    );
    assert!(processed.await.is_err());
}

// oracle: mutant retaining a stale clipboard event or leaving its completion open.
#[tokio::test]
async fn stale_queued_completion_event_is_closed() {
    let mut server = server();
    let (event, processed) = completion_event(copy_message());
    server.event_sender().send(event).unwrap();

    server.discard_stale_session_events().await;

    assert!(processed.await.is_err());
}

// oracle: mutant leaving missing-channel completions open or writing clipboard data.
#[tokio::test]
async fn missing_clipboard_channel_completes_events_without_output() {
    let (first, first_processed) = completion_event(copy_message());
    let (second, second_processed) = completion_event(copy_message());
    let bytes = dispatch_and_capture(vec![first, second], StaticChannelSet::new()).await;
    assert!(bytes.is_empty());
    assert_eq!(first_processed.await, Ok(()));
    assert_eq!(second_processed.await, Ok(()));
}

// oracle: mutant return an SVC channel error for a registered-but-unjoined clipboard processor.
#[tokio::test]
async fn unjoined_clipboard_channel_completes_without_output() {
    let (event, processed) = completion_event(copy_message());
    let bytes = dispatch_and_capture(vec![event], unjoined_clipboard_channel()).await;

    assert!(bytes.is_empty());
    assert_eq!(processed.await, Ok(()));
}

// oracle: mutant writing output for a refused request or preventing the later event from running.
#[tokio::test]
async fn refused_request_completes_before_a_later_event() {
    let (refused, refused_processed) =
        completion_event(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT));
    let (later, later_processed) = completion_event(copy_message());
    let (stream, mut peer) = tokio::io::duplex(8192);
    let mut transport = TokioFramed::new(stream);
    let mut server = server();
    let mut events = vec![refused, later];
    dispatch_static_channel_events(
        &mut server,
        clipboard_channels(),
        &mut events,
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    assert_eq!(refused_processed.await, Ok(()));
    assert_eq!(later_processed.await, Ok(()));
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(frame_count(&bytes), 1, "only the later event should produce output");
}

// oracle: mutant turning a backend error notification into a session error.
#[tokio::test]
async fn clipboard_error_completes_without_session_error() {
    let message = ClipboardMessage::Error(Box::new(std::io::Error::other("backend failure")));
    let (event, processed) = completion_event(message);
    dispatch_and_capture(vec![event], clipboard_channels()).await;

    assert_eq!(processed.await, Ok(()));
}

// oracle: mutant propagating a failed completion send as a dispatch error.
#[tokio::test]
async fn dropping_completion_receiver_does_not_fail_dispatch() {
    let (event, processed) = completion_event(copy_message());
    drop(processed);
    let bytes = dispatch_and_capture(vec![event], clipboard_channels()).await;

    assert_eq!(frame_count(&bytes), 1);
}

// oracle: mutant changing legacy clipboard handling when adding completion.
#[tokio::test]
async fn legacy_and_completion_events_emit_equivalent_output() {
    let legacy = dispatch_and_capture(vec![ServerEvent::Clipboard(copy_message())], clipboard_channels()).await;
    let (completed_event, processed) = completion_event(copy_message());
    let completed = dispatch_and_capture(vec![completed_event], clipboard_channels()).await;

    assert_eq!(processed.await, Ok(()));
    assert_eq!(completed, legacy);
}

// oracle: mutant exposing clipboard payload or completion sender internals in Debug.
#[test]
fn completion_event_debug_redacts_message_and_sender() {
    let message = ClipboardMessage::SendFormatData(OwnedFormatDataResponse::new_data(
        b"clipboard completion private data".as_slice(),
    ));
    let message_debug = format!("{message:?}");
    let (event, _processed) = completion_event(message);
    let debug = format!("{event:?}");

    assert!(!debug.contains("Sender"), "Debug exposed completion sender details");
    assert!(
        !debug.contains(&message_debug),
        "Debug exposed the clipboard message and payload"
    );
}
