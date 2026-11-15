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
use ironrdp_pdu::rdp::vc::ChannelPduHeader;
use ironrdp_pdu::x224::X224;
use ironrdp_rdpsnd::pdu::{
    AudioFormat, AudioFormatFlags, ClientAudioFormatPdu, ClientAudioOutputPdu, QualityMode, QualityModePdu,
    ServerAudioOutputPdu, TrainingConfirmPdu, Version, WaveFormat,
};
use ironrdp_rdpsnd::server::{NegotiatedFormat, RdpsndError, RdpsndServer, RdpsndServerHandler, RdpsndServerMessage};
use ironrdp_server::{ConnectionWriter, RdpServer, ServerEvent, dispatch_static_channel_events};
use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};
use ironrdp_tokio::TokioFramed;
use tokio::io::AsyncReadExt as _;
use tokio::sync::{Semaphore, oneshot};

const CLIPBOARD_CHANNEL_ID: u16 = 1004;
const RDPSND_CHANNEL_ID: u16 = 1005;

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

    let (completion, processed) = oneshot::channel();
    let valid_event = ServerEvent::ClipboardWithValidity {
        message: ClipboardMessage::SendInitiateCopy(vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]),
        completion,
        is_current: Box::new(|| true),
    };
    let (stream, mut peer) = tokio::io::duplex(8192);
    let mut transport = TokioFramed::new(stream);
    let mut writer = ConnectionWriter::new(&mut transport);
    let mut server = server();
    let mut events = vec![valid_event];
    let _channels = dispatch_static_channel_events(&mut server, channels, &mut events, &mut writer)
        .await
        .unwrap();
    assert_eq!(processed.await, Ok(()));
    drop(transport);
    let mut valid_bytes = Vec::new();
    peer.read_to_end(&mut valid_bytes).await.unwrap();
    assert_eq!(count_frames(&valid_bytes), 1);
}

#[derive(Debug)]
struct StaticPcmHandler(Vec<AudioFormat>);

impl RdpsndServerHandler for StaticPcmHandler {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.0
    }

    fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat> {
        common.first()
    }

    fn start(&mut self, _format: &NegotiatedFormat) -> Result<(), Box<dyn RdpsndError>> {
        Ok(())
    }

    fn stop(&mut self) {}
}

fn negotiated_rdpsnd_channels() -> StaticChannelSet {
    let format = AudioFormat {
        format: WaveFormat::PCM,
        n_channels: 2,
        n_samples_per_sec: 44_100,
        n_avg_bytes_per_sec: 176_400,
        n_block_align: 4,
        bits_per_sample: 16,
        data: None,
    };
    let mut rdpsnd = RdpsndServer::new(Box::new(StaticPcmHandler(vec![format.clone()])));
    rdpsnd.start().unwrap();
    let client_formats = ClientAudioOutputPdu::AudioFormat(ClientAudioFormatPdu {
        version: Version::V8,
        flags: AudioFormatFlags::ALIVE | AudioFormatFlags::VOLUME,
        formats: vec![format],
        volume_left: 0,
        volume_right: 0,
        pitch: 0,
        dgram_port: 0,
    });
    rdpsnd.process(&encode_vec(&client_formats).unwrap()).unwrap();
    let quality = ClientAudioOutputPdu::QualityMode(QualityModePdu {
        quality_mode: QualityMode::High,
    });
    rdpsnd.process(&encode_vec(&quality).unwrap()).unwrap();
    let confirm = ClientAudioOutputPdu::TrainingConfirm(TrainingConfirmPdu {
        timestamp: 0,
        pack_size: 0,
    });
    rdpsnd.process(&encode_vec(&confirm).unwrap()).unwrap();

    let mut channels = StaticChannelSet::new();
    channels.insert(rdpsnd);
    channels.attach_channel_id(TypeId::of::<RdpsndServer>(), RDPSND_CHANNEL_ID);
    channels
}

fn decode_wave_timestamps(bytes: &[u8]) -> Vec<u16> {
    let mut frames = ReadCursor::new(bytes);
    let mut timestamps = Vec::new();
    while !frames.is_empty() {
        let frame: X224<SendDataIndication<'_>> = decode_cursor(&mut frames).unwrap();
        let mut payload = ReadCursor::new(frame.0.user_data.as_ref());
        let _: ChannelPduHeader = decode_cursor(&mut payload).unwrap();
        if let ServerAudioOutputPdu::Wave2(wave) = decode_cursor(&mut payload).unwrap() {
            timestamps.push(wave.timestamp);
        }
    }
    timestamps
}

// oracle: mutant counting stale RdpsndWithValidity waves against WAVE_KEEP or releasing permits before dispatch/discard.
#[tokio::test]
async fn stale_rdpsnd_waves_preserve_wave_keep_budget_and_release_permits() {
    let channels = negotiated_rdpsnd_channels();
    let permits = Arc::new(Semaphore::new(2));

    let blocker = ServerEvent::Rdpsnd(RdpsndServerMessage::SetVolume {
        left: 0xFFFF,
        right: 0xFFFF,
    });
    let mid_batch_current = Arc::new(AtomicBool::new(true));
    let mid_batch_flag = Arc::clone(&mid_batch_current);
    let invalidated_mid_batch = ServerEvent::RdpsndWithValidity {
        message: RdpsndServerMessage::Wave(vec![0; 4], 99),
        permit: Some(Arc::clone(&permits).try_acquire_owned().unwrap()),
        is_current: Box::new(move || mid_batch_flag.load(Ordering::Acquire)),
    };
    let current_waves = [10, 20, 30, 40].map(|ts| ServerEvent::RdpsndWithValidity {
        message: RdpsndServerMessage::Wave(vec![0; 4], ts),
        permit: None,
        is_current: Box::new(|| true),
    });
    let trailing_stale = ServerEvent::RdpsndWithValidity {
        message: RdpsndServerMessage::Wave(vec![0; 4], 98),
        permit: Some(Arc::clone(&permits).try_acquire_owned().unwrap()),
        is_current: Box::new(|| false),
    };

    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut bytes = Vec::new();
    let channels = {
        let mut writer = ConnectionWriter::new(&mut transport);
        let mut server = server();
        let mut events = vec![blocker, invalidated_mid_batch];
        events.extend(current_waves);
        events.push(trailing_stale);
        let dispatch = dispatch_static_channel_events(&mut server, channels, &mut events, &mut writer);
        tokio::pin!(dispatch);
        assert_pending(dispatch.as_mut()).await;
        assert_eq!(permits.available_permits(), 0);
        mid_batch_current.store(false, Ordering::Release);

        drain_write(&mut peer, &mut bytes, dispatch.as_mut()).await
    };
    assert_eq!(permits.available_permits(), 2);
    drop(transport);
    peer.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(decode_wave_timestamps(&bytes), vec![10, 20, 30, 40]);

    let wave_permits = Arc::new(Semaphore::new(1));
    let stalled_wave = ServerEvent::RdpsndWithValidity {
        message: RdpsndServerMessage::Wave(vec![0; 4], 50),
        permit: Some(Arc::clone(&wave_permits).try_acquire_owned().unwrap()),
        is_current: Box::new(|| true),
    };
    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut wave_bytes = Vec::new();
    {
        let mut writer = ConnectionWriter::new(&mut transport);
        let mut server = server();
        let mut events = vec![stalled_wave];
        let dispatch = dispatch_static_channel_events(&mut server, channels, &mut events, &mut writer);
        tokio::pin!(dispatch);
        assert_pending(dispatch.as_mut()).await;
        assert_eq!(wave_permits.available_permits(), 0);

        drain_write(&mut peer, &mut wave_bytes, dispatch.as_mut()).await;
    }
    assert_eq!(wave_permits.available_permits(), 1);
    drop(transport);
    peer.read_to_end(&mut wave_bytes).await.unwrap();
    assert_eq!(decode_wave_timestamps(&wave_bytes), vec![50]);
}
