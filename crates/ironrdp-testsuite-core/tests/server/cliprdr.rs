use core::any::TypeId;
use core::net::Ipv4Addr;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::Poll;
use core::time::Duration;
use std::sync::Arc;

use ironrdp_cliprdr::CliprdrServer;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, ClipboardPdu,
    ClipboardProtocolVersion, FileContentsRequest, FileContentsResponse, FileDescriptor, FormatDataRequest,
    FormatDataResponse, FormatList, LockDataId,
};
use ironrdp_core::{ReadCursor, decode_cursor, encode_vec, impl_as_any};
use ironrdp_pdu::mcs::SendDataIndication;
use ironrdp_pdu::x224::X224;
use ironrdp_server::{ConnectionWriter, RdpServer, ServerEvent, dispatch_static_channel_events};
use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};
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

async fn assert_pending(future: Pin<&mut impl Future>) {
    let mut future = future;
    core::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

async fn drain_write<T>(
    peer: &mut tokio::io::DuplexStream,
    bytes: &mut Vec<u8>,
    mut dispatch: Pin<&mut impl Future<Output = ironrdp_server::ServerResult<T>>>,
) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                sent = &mut dispatch => break sent.unwrap(),
                read = peer.read_buf(bytes) => assert_ne!(read.unwrap(), 0),
            }
        }
    })
    .await
    .unwrap()
}

fn count_frames(bytes: &[u8]) -> usize {
    let mut frames = ReadCursor::new(bytes);
    let mut count = 0;
    while !frames.is_empty() {
        let _: X224<SendDataIndication<'_>> = decode_cursor(&mut frames).unwrap();
        count += 1;
    }
    count
}

// oracle: mutant writing or mutating CLIPRDR state when a guarded clipboard offer is invalidated behind an earlier event.
#[tokio::test]
async fn invalidated_clipboard_offer_is_discarded_after_preceding_event() {
    let mut cliprdr = CliprdrServer::new(Box::new(SilentBackend));
    cliprdr.start().unwrap();
    let client_caps = ClipboardPdu::Capabilities(Capabilities::new(
        ClipboardProtocolVersion::V2,
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
    ));
    cliprdr.process(&encode_vec(&client_caps).unwrap()).unwrap();
    let initial_formats = ClipboardPdu::FormatList(FormatList::new_unicode(&[], true).unwrap());
    cliprdr.process(&encode_vec(&initial_formats).unwrap()).unwrap();

    let mut channels = StaticChannelSet::new();
    channels.insert(cliprdr);
    channels.attach_channel_id(TypeId::of::<CliprdrServer>(), CLIPBOARD_CHANNEL_ID);

    let blocker = ServerEvent::Clipboard(ClipboardMessage::SendInitiateFileCopy(vec![FileDescriptor::new(
        "active.txt",
    )]));
    let current = Arc::new(AtomicBool::new(true));
    let event_current = Arc::clone(&current);
    let (completion, mut processed) = oneshot::channel();
    let event = ServerEvent::ClipboardWithValidity {
        message: ClipboardMessage::SendInitiateCopy(vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]),
        completion,
        is_current: Box::new(move || event_current.load(Ordering::Acquire)),
    };

    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut bytes = Vec::new();
    let mut channels = {
        let mut writer = ConnectionWriter::new(&mut transport);
        let mut server = server();
        let mut events = vec![blocker, event];
        let dispatch = dispatch_static_channel_events(&mut server, channels, &mut events, &mut writer);
        tokio::pin!(dispatch);
        assert_pending(dispatch.as_mut()).await;
        assert_eq!(processed.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        current.store(false, Ordering::Release);

        drain_write(&mut peer, &mut bytes, dispatch.as_mut()).await
    };
    drop(transport);
    peer.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(
        count_frames(&bytes),
        1,
        "invalidated clipboard offer escaped after preceding event"
    );
    assert!(
        processed.await.is_err(),
        "discarded clipboard offer signaled completion"
    );

    // `initiate_copy` clears `local_file_list`; verify the discarded offer left the file list intact.
    let file_list_request = ClipboardPdu::FormatDataRequest(FormatDataRequest {
        format: ClipboardFormatId(0xC0FE),
    });
    let reply = channels
        .get_by_type_mut::<CliprdrServer>()
        .unwrap()
        .channel_processor_downcast_mut::<CliprdrServer>()
        .unwrap()
        .process(&encode_vec(&file_list_request).unwrap())
        .unwrap();
    assert_eq!(reply.len(), 1, "stored file list was cleared by a discarded offer");
}
