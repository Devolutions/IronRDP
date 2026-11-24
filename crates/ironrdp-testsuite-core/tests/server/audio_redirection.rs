use core::any::TypeId;
use core::net::Ipv4Addr;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::Poll;
use core::time::Duration;
use std::sync::{Arc, Mutex};

use ironrdp_core::{encode_vec, impl_as_any};
use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
use ironrdp_rdpsnd::pdu::{
    AudioFormat, AudioFormatFlags, ClientAudioFormatPdu, ClientAudioOutputPdu, TrainingConfirmPdu, Version, WaveFormat,
};
use ironrdp_rdpsnd::server::{NegotiatedFormat, RdpsndError, RdpsndServer, RdpsndServerHandler, RdpsndServerMessage};
use ironrdp_server::{
    OutboundQueue, QueuedWriter, RdpServer, ServerEvent, audio_redirection_wanted, dispatch_static_channel_events,
    drive_static_channel_events,
};
use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};
use ironrdp_tokio::TokioFramed;
use tokio::io::AsyncReadExt as _;
use tokio::sync::Semaphore;

/// Per MS-RDPBCGR [2.2.1.11.1.1] Info Packet (TS_INFO_PACKET), audio is redirected to the client only when its flags
/// set neither INFO_NOAUDIOPLAYBACK, under which audio redirection MUST NOT take place, nor INFO_REMOTECONSOLEAUDIO,
/// which asks for the audio to be played on the server.
/// Other flags, INFO_AUDIOCAPTURE included, don't change that: "If the INFO_NOAUDIOPLAYBACK flag is not set, then audio
/// redirection SHOULD take place if the INFO_REMOTECONSOLEAUDIO (0x00002000) flag is also not set."
///
/// [2.2.1.11.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/732394f5-e2b5-4ac5-8a0a-35345386b0d1
// oracle: spec MS-RDPBCGR §2.2.1.11.1.1
#[rstest::rstest]
#[case::neither_audio_flag(ClientInfoFlags::empty(), true)]
#[case::no_audio_playback(ClientInfoFlags::NO_AUDIO_PLAYBACK, false)]
#[case::remote_console_audio(ClientInfoFlags::REMOTE_CONSOLE_AUDIO, false)]
#[case::both_audio_flags(ClientInfoFlags::NO_AUDIO_PLAYBACK | ClientInfoFlags::REMOTE_CONSOLE_AUDIO, false)]
#[case::audio_capture_only(ClientInfoFlags::AUDIO_CAPTURE, true)]
fn audio_redirection_follows_the_info_packet_audio_flags(#[case] flags: ClientInfoFlags, #[case] redirected: bool) {
    assert_eq!(audio_redirection_wanted(flags), redirected);
}

#[derive(Debug)]
struct WaveCounter {
    formats: Vec<AudioFormat>,
    unconfirmed: Arc<Mutex<Vec<usize>>>,
}

impl_as_any!(WaveCounter);

impl RdpsndServerHandler for WaveCounter {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat> {
        common.first()
    }

    fn start(&mut self, _: &NegotiatedFormat) -> Result<(), Box<dyn RdpsndError>> {
        Ok(())
    }

    fn stop(&mut self) {}

    fn unconfirmed_waves_changed(&mut self, unconfirmed: usize) {
        self.unconfirmed.lock().unwrap().push(unconfirmed);
    }
}

fn ready_rdpsnd() -> (StaticChannelSet, Arc<Mutex<Vec<usize>>>) {
    let format = AudioFormat {
        format: WaveFormat::PCM,
        n_channels: 2,
        n_samples_per_sec: 44_100,
        n_avg_bytes_per_sec: 176_400,
        n_block_align: 4,
        bits_per_sample: 16,
        data: None,
    };
    let unconfirmed = Arc::new(Mutex::new(Vec::new()));
    let mut rdpsnd = RdpsndServer::new(Box::new(WaveCounter {
        formats: vec![format.clone()],
        unconfirmed: Arc::clone(&unconfirmed),
    }));
    rdpsnd.start().unwrap();
    let client_formats = ClientAudioOutputPdu::AudioFormat(ClientAudioFormatPdu {
        version: Version::V5,
        flags: AudioFormatFlags::VOLUME,
        formats: vec![format],
        volume_left: 0,
        volume_right: 0,
        pitch: 0,
        dgram_port: 0,
    });
    rdpsnd.process(&encode_vec(&client_formats).unwrap()).unwrap();
    let confirm = ClientAudioOutputPdu::TrainingConfirm(TrainingConfirmPdu {
        timestamp: 0,
        pack_size: 0,
    });
    rdpsnd.process(&encode_vec(&confirm).unwrap()).unwrap();

    let mut channels = StaticChannelSet::new();
    channels.insert(rdpsnd);
    channels.attach_channel_id(TypeId::of::<RdpsndServer>(), 1005);
    (channels, unconfirmed)
}

fn server() -> RdpServer {
    RdpServer::builder()
        .with_addr((Ipv4Addr::LOCALHOST, 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build()
}

// oracle: mutant releasing RdpsndWithValidity's permit on queue before the transport write completes.
#[tokio::test]
async fn rdpsnd_validity_retains_permit_until_transport_write_completes() {
    let (channels, _) = ready_rdpsnd();
    let permits = Arc::new(Semaphore::new(1));
    let event = ServerEvent::RdpsndWithValidity {
        message: RdpsndServerMessage::Wave(vec![0; 8], 100),
        permit: Some(Arc::clone(&permits).try_acquire_owned().unwrap()),
        is_current: Box::new(|| true),
    };
    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let queue = OutboundQueue::default();
    let mut writer = QueuedWriter::new(&queue);
    let mut events = vec![event];
    dispatch_static_channel_events(&mut server(), channels, &mut events, &mut writer)
        .await
        .unwrap();
    queue.finish();
    assert_eq!(permits.available_permits(), 0, "permit released before transport write");

    let mut bytes = Vec::new();
    let writing = queue.write_queued(&mut transport);
    tokio::pin!(writing);
    core::future::poll_fn(|cx| {
        assert!(writing.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(permits.available_permits(), 0, "permit released mid-write");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                res = &mut writing => break res.unwrap(),
                read = peer.read_buf(&mut bytes) => assert_ne!(read.unwrap(), 0),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(permits.available_permits(), 1);
}

// oracle: mutant dispatching or counting a wave invalidated behind an earlier stalled write.
#[tokio::test]
async fn invalidated_wave_behind_stalled_write_neither_sends_nor_consumes_wave_keep() {
    let (channels, unconfirmed) = ready_rdpsnd();
    let current = Arc::new(AtomicBool::new(true));
    let event_current = Arc::clone(&current);
    let permits = Arc::new(Semaphore::new(1));
    let mut events = vec![
        ServerEvent::Rdpsnd(RdpsndServerMessage::SetVolume {
            left: 0xFFFF,
            right: 0xFFFF,
        }),
        ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(vec![1; 8], 200)),
        ServerEvent::RdpsndWithValidity {
            message: RdpsndServerMessage::Wave(vec![9; 8], 250),
            permit: Some(Arc::clone(&permits).try_acquire_owned().unwrap()),
            is_current: Box::new(move || event_current.load(Ordering::Acquire)),
        },
        ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(vec![2; 8], 300)),
        ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(vec![3; 8], 400)),
        ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(vec![4; 8], 500)),
    ];

    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut server = server();
    let mut bytes = Vec::new();
    {
        let dispatch = drive_static_channel_events(&mut server, channels, &mut events, &mut transport);
        tokio::pin!(dispatch);
        core::future::poll_fn(|cx| {
            assert!(dispatch.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        current.store(false, Ordering::Release);

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    res = &mut dispatch => break res.unwrap(),
                    read = peer.read_buf(&mut bytes) => assert_ne!(read.unwrap(), 0),
                }
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(permits.available_permits(), 1);
    assert_eq!(
        *unconfirmed.lock().unwrap(),
        vec![1, 2, 3, 4],
        "all 4 remaining current waves must be sent without the invalidated wave consuming a WAVE_KEEP slot"
    );
}
