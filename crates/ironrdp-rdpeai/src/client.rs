//! AUDIO_INPUT dynamic virtual channel client (MS-RDPEAI).

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
pub use ironrdp_pdu::PduResult;
use ironrdp_pdu::{decode_err, encode_err, pdu_other_err};
use ironrdp_rdpsnd::pdu::AudioFormat;
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, info, trace, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{DataPdu, FormatChangePdu, FormatsPdu, OpenPdu, OpenReplyPdu, RdpeaiPdu, VersionPdu};

/// Callback used to push already-wrapped DRDYNVC SVC messages into the session loop.
pub type DvcUplink = Box<dyn FnMut(u32, Vec<SvcMessage>) -> PduResult<()> + Send>;

/// Sink for one complete capture packet (raw audio bytes, no RDPEAI header).
pub type AudioPacketSink = Box<dyn FnMut(Vec<u8>) + Send>;

/// Observer for failures to encode or enqueue captured audio.
///
/// The observer may run on an audio callback thread and must not block.
pub type UplinkErrorHandler = Arc<dyn Fn(String) + Send + Sync>;

/// Capture device backend driven by the AUDIO_INPUT client processor.
pub trait RdpeaiCaptureHandler: Send {
    /// Formats this backend can negotiate as encoding formats, in preferred order.
    fn supported_formats(&self) -> &[AudioFormat];

    /// Chooses the encoding formats to offer in reply to the server's Sound Formats PDU.
    ///
    /// The order states the client's preference. The client keeps only the returned formats the
    /// server offered, and Open and Format Change PDUs index that list. The default returns
    /// [`supported_formats`](Self::supported_formats).
    fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
        let _ = server_formats;
        self.supported_formats().to_vec()
    }

    /// Open capture for `capture_format` and begin delivering fixed-size packets to `sink`.
    ///
    /// `encode_format` is the negotiated encoding format selected by Open `initialFormat`.
    /// It may differ from `capture_format` when the backend captures PCM and encodes later.
    /// `packet_size` is the Open capture Data PDU size (`nChannels * 2 * FramesPerPacket`).
    ///
    /// Returns an HRESULT (`0` = success).
    fn open(
        &mut self,
        capture_format: &AudioFormat,
        encode_format: &AudioFormat,
        packet_size: usize,
        sink: AudioPacketSink,
    ) -> i32;

    /// Switch the negotiated encoding format while capture stays open.
    ///
    /// `packet_size` remains the capture packet size established by Open; it must not be
    /// recalculated from `encode_format`.
    ///
    /// PCM-only backends may restart the capture device when `encode_format` differs from
    /// the Open capture WAVEFORMATEX so FormatChange can be confirmed honestly.
    fn set_format(&mut self, encode_format: &AudioFormat, packet_size: usize) -> bool;

    /// Switch formats and enqueue the confirmation at the boundary between old and new packets.
    ///
    /// Implementations must serialize `confirm` with delivery to the packet sink.
    /// A successful switch must call `confirm` exactly once before delivering new-format packets.
    /// The default rejects ordered changes rather than allowing packets to overtake the confirmation.
    fn set_format_ordered(
        &mut self,
        _encode_format: &AudioFormat,
        _packet_size: usize,
        _confirm: &mut dyn FnMut() -> PduResult<()>,
    ) -> PduResult<bool> {
        Err(pdu_other_err!("ordered audio format changes are unsupported"))
    }

    /// Stop capture and release the device.
    fn close(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Waiting for server Version PDU.
    WaitVersion,
    /// Waiting for server Formats PDU.
    WaitFormats,
    /// Formats exchanged; waiting for Open or FormatChange.
    Ready,
    /// Capture device open; streaming Data PDUs.
    Opened,
}

/// Client processor for the `AUDIO_INPUT` dynamic virtual channel.
pub struct RdpeaiClient {
    handler: Box<dyn RdpeaiCaptureHandler>,
    uplink: Arc<Mutex<DvcUplink>>,
    state: State,
    channel_id: Option<u32>,
    /// Negotiated format list (client reply order); Open/FormatChange indices refer here.
    negotiated_formats: Vec<AudioFormat>,
    frames_per_packet: u32,
    /// Capture Data PDU payload size from the last successful Open.
    capture_packet_size: Option<usize>,
    /// Drop-flag for in-flight capture callbacks after close/reopen.
    capture_epoch: Arc<AtomicU32>,
    ordered_replies: bool,
    pending_open: Option<Arc<AtomicBool>>,
    uplink_error_handler: Option<UplinkErrorHandler>,
}

impl RdpeaiClient {
    /// Create a client. `uplink` delivers continuous capture traffic after Open.
    pub fn new(handler: Box<dyn RdpeaiCaptureHandler>, uplink: DvcUplink) -> Self {
        Self {
            handler,
            uplink: Arc::new(Mutex::new(uplink)),
            state: State::WaitVersion,
            channel_id: None,
            negotiated_formats: Vec::new(),
            frames_per_packet: 0,
            capture_packet_size: None,
            capture_epoch: Arc::new(AtomicU32::new(0)),
            ordered_replies: false,
            pending_open: None,
            uplink_error_handler: None,
        }
    }

    /// Enqueue control replies with capture packets instead of returning them directly.
    ///
    /// Samples received before the Open reply is queued are discarded rather than sent ahead of it.
    /// Capture handlers must implement [`RdpeaiCaptureHandler::set_format_ordered`].
    #[must_use]
    pub fn with_ordered_replies(mut self) -> Self {
        self.ordered_replies = true;
        self
    }

    /// Observe failures from the asynchronous capture uplink.
    #[must_use]
    pub fn with_uplink_error_handler(mut self, handler: UplinkErrorHandler) -> Self {
        self.uplink_error_handler = Some(handler);
        self
    }

    fn build_packet_sink(&self) -> AudioPacketSink {
        let channel_id = self.channel_id.expect("open only after start");
        let uplink = Arc::clone(&self.uplink);
        let epoch = Arc::clone(&self.capture_epoch);
        let expected = epoch.load(Ordering::Acquire);
        let open = self.pending_open.clone();
        let error_handler = self.uplink_error_handler.clone();

        Box::new(move |packet: Vec<u8>| {
            if epoch.load(Ordering::Acquire) != expected
                || open.as_ref().is_some_and(|open| !open.load(Ordering::Acquire))
            {
                return;
            }
            let messages: Vec<DvcMessage> = vec![
                Box::new(RdpeaiPdu::DataIncoming),
                Box::new(RdpeaiPdu::Data(DataPdu::new(packet))),
            ];
            let svc = match encode_dvc_messages(channel_id, messages, ChannelFlags::empty()) {
                Ok(v) => v,
                Err(error) => {
                    warn!(%error, "Failed to encode AUDIO_INPUT data PDUs");
                    if let Some(handler) = &error_handler {
                        handler(format!("failed to encode audio capture data: {error}"));
                    }
                    return;
                }
            };
            let Ok(mut uplink) = uplink.lock() else {
                if let Some(handler) = &error_handler {
                    handler("audio capture uplink lock poisoned".to_owned());
                }
                return;
            };
            if let Err(error) = uplink(channel_id, svc) {
                warn!(%error, "Failed to uplink AUDIO_INPUT data");
                if let Some(handler) = &error_handler {
                    handler(format!("failed to enqueue audio capture data: {error}"));
                }
            }
        })
    }

    fn resolve_format(&self, index: u32) -> Option<&AudioFormat> {
        let idx = usize::try_from(index).ok()?;
        self.negotiated_formats.get(idx)
    }

    fn handle_version(&mut self, pdu: VersionPdu) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::WaitVersion {
            // Out-of-sequence Version must not reset an opened/ready client.
            warn!(?self.state, "Ignoring out-of-sequence Version PDU");
            return Ok(Vec::new());
        }
        let client_version = pdu.version;
        self.state = State::WaitFormats;
        debug!(?client_version, "AUDIO_INPUT version negotiated");
        Ok(vec![Box::new(RdpeaiPdu::Version(VersionPdu::new(client_version)))])
    }

    fn handle_formats(&mut self, server: FormatsPdu) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::WaitFormats && self.state != State::Ready && self.state != State::Opened {
            warn!(?self.state, "Unexpected Formats PDU");
        }

        if self.state == State::Opened {
            self.stop_capture();
        }

        debug!(server_formats = ?server.formats, "AUDIO_INPUT server formats");
        let negotiated =
            ironrdp_rdpsnd::pdu::retain_offered(self.handler.choose_formats(&server.formats), &server.formats);

        if negotiated.is_empty() {
            warn!(server_formats = server.formats.len(), "No common AUDIO_INPUT formats");
        } else {
            info!(count = negotiated.len(), "AUDIO_INPUT formats negotiated");
        }

        self.negotiated_formats = negotiated.clone();
        self.state = State::Ready;

        // MS-RDPEAI §3.2.5.1.4–5: Incoming Data PDU, then client Sound Formats PDU.
        Ok(vec![
            Box::new(RdpeaiPdu::DataIncoming),
            Box::new(RdpeaiPdu::Formats(FormatsPdu::client(negotiated))),
        ])
    }

    fn handle_open(&mut self, open: OpenPdu) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::Ready && self.state != State::Opened {
            warn!(?self.state, "Open PDU outside Ready/Opened");
        }

        if self.state == State::Opened {
            self.stop_capture();
        }

        let Some(encode_fmt) = self.resolve_format(open.initial_format).cloned() else {
            warn!(index = open.initial_format, "Open initialFormat out of range");
            return Ok(vec![Box::new(RdpeaiPdu::OpenReply(OpenReplyPdu::fail()))]);
        };

        // Capture WAVEFORMATEX from Open is independent of initialFormat (encoding).
        let capture_fmt = open.capture_format.clone();

        let Some(packet_size) = open.data_packet_size() else {
            warn!(
                frames_per_packet = open.frames_per_packet,
                "Open FramesPerPacket rejected"
            );
            return Ok(vec![Box::new(RdpeaiPdu::OpenReply(OpenReplyPdu::fail()))]);
        };

        self.frames_per_packet = open.frames_per_packet;

        // Confirm initial format before Open Reply (MS-RDPEAI §3.2.5.1.7).
        let mut out: Vec<DvcMessage> = vec![Box::new(RdpeaiPdu::FormatChange(FormatChangePdu::new(
            open.initial_format,
        )))];

        if self.ordered_replies {
            self.pending_open = Some(Arc::new(AtomicBool::new(false)));
        }
        let sink = self.build_packet_sink();
        let hr = self.handler.open(&capture_fmt, &encode_fmt, packet_size, sink);
        if hr == OpenReplyPdu::S_OK {
            self.capture_packet_size = Some(packet_size);
            self.state = State::Opened;
            debug!(
                format_idx = open.initial_format,
                packet_size, "AUDIO_INPUT capture opened"
            );
            out.push(Box::new(RdpeaiPdu::OpenReply(OpenReplyPdu::ok())));
        } else {
            self.capture_packet_size = None;
            self.state = State::Ready;
            warn!(hr, "AUDIO_INPUT capture open failed");
            out.push(Box::new(RdpeaiPdu::OpenReply(OpenReplyPdu { result: hr })));
        }
        Ok(out)
    }

    fn handle_format_change(&mut self, change: FormatChangePdu) -> PduResult<Vec<DvcMessage>> {
        let Some(encode_fmt) = self.resolve_format(change.new_format).cloned() else {
            warn!(index = change.new_format, "FormatChange index out of range");
            return Ok(Vec::new());
        };

        if self.state != State::Opened {
            // Acknowledge selection while idle; device is not capturing yet.
            return Ok(vec![Box::new(RdpeaiPdu::FormatChange(FormatChangePdu::new(
                change.new_format,
            )))]);
        }

        // FormatChange switches encoding only; capture packet size stays from Open.
        // MS-RDPEAI §3.2.5.3.1–2: the client always confirms a valid FormatChange.
        let Some(packet_size) = self.capture_packet_size else {
            warn!(index = change.new_format, "FormatChange without Open packet size");
            self.stop_capture();
            return Ok(vec![Box::new(RdpeaiPdu::FormatChange(FormatChangePdu::new(
                change.new_format,
            )))]);
        };

        if self.ordered_replies {
            let uplink = Arc::clone(&self.uplink);
            let channel_id = self
                .channel_id
                .ok_or_else(|| pdu_other_err!("AUDIO_INPUT channel not started"))?;
            let confirmed = Cell::new(false);
            let mut confirm = || {
                let messages: Vec<DvcMessage> = vec![Box::new(RdpeaiPdu::FormatChange(FormatChangePdu::new(
                    change.new_format,
                )))];
                queue_messages(&uplink, channel_id, messages)?;
                confirmed.set(true);
                Ok(())
            };
            let changed = self.handler.set_format_ordered(&encode_fmt, packet_size, &mut confirm);
            if !matches!(changed, Ok(true)) {
                self.stop_capture();
            }
            if changed? && !confirmed.get() {
                self.stop_capture();
                return Err(pdu_other_err!("audio format changed without confirming the transition"));
            }
            if !confirmed.get() {
                confirm()?;
            }
            return Ok(Vec::new());
        }

        if self.handler.set_format(&encode_fmt, packet_size) {
            // Data PDUs reach the server through the uplink queue. Queuing the confirmation there too
            // keeps it behind the packets encoded in the old format, which the server would
            // otherwise decode with the new one.
            match self.queue_behind_data(RdpeaiPdu::FormatChange(FormatChangePdu::new(change.new_format))) {
                Ok(()) => return Ok(Vec::new()),
                Err(error) => warn!(%error, "Failed to queue the FormatChange confirmation; replying directly"),
            }
        } else {
            warn!(index = change.new_format, "Capture backend rejected format change");
            // Still confirm so the server does not hang Opened with no timeout.
            self.stop_capture();
        }

        Ok(vec![Box::new(RdpeaiPdu::FormatChange(FormatChangePdu::new(
            change.new_format,
        )))])
    }

    /// Sends `pdu` through the uplink, after the capture packets already queued there.
    fn queue_behind_data(&self, pdu: RdpeaiPdu) -> PduResult<()> {
        let channel_id = self
            .channel_id
            .ok_or_else(|| pdu_other_err!("AUDIO_INPUT channel not started"))?;
        queue_messages(&self.uplink, channel_id, vec![Box::new(pdu)])
    }

    fn stop_capture(&mut self) {
        self.capture_epoch.fetch_add(1, Ordering::AcqRel);
        self.handler.close();
        self.capture_packet_size = None;
        self.pending_open = None;
        self.state = State::Ready;
    }
}

fn queue_messages(uplink: &Arc<Mutex<DvcUplink>>, channel_id: u32, messages: Vec<DvcMessage>) -> PduResult<()> {
    let svc = encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|e| encode_err!(e))?;
    let mut uplink = uplink
        .lock()
        .map_err(|_| pdu_other_err!("AUDIO_INPUT uplink lock poisoned"))?;
    uplink(channel_id, svc)
}

impl_as_any!(RdpeaiClient);

impl DvcProcessor for RdpeaiClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        self.state = State::WaitVersion;
        self.negotiated_formats.clear();
        self.frames_per_packet = 0;
        self.capture_packet_size = None;
        self.pending_open = None;
        debug!(channel_id, "AUDIO_INPUT channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: RdpeaiPdu = decode(payload).map_err(|e| decode_err!(e))?;
        trace!(?pdu, "AUDIO_INPUT PDU received");
        let replies = match pdu {
            RdpeaiPdu::Version(v) => self.handle_version(v),
            RdpeaiPdu::Formats(f) => self.handle_formats(f),
            RdpeaiPdu::Open(o) => self.handle_open(o),
            RdpeaiPdu::FormatChange(c) => self.handle_format_change(c),
            RdpeaiPdu::DataIncoming | RdpeaiPdu::Data(_) | RdpeaiPdu::OpenReply(_) => {
                warn!("Ignoring client-originated AUDIO_INPUT PDU from server");
                Ok(Vec::new())
            }
        }?;
        if !self.ordered_replies || replies.is_empty() {
            return Ok(replies);
        }
        let channel_id = self
            .channel_id
            .ok_or_else(|| pdu_other_err!("AUDIO_INPUT channel not started"))?;
        if let Err(error) = queue_messages(&self.uplink, channel_id, replies) {
            self.stop_capture();
            return Err(error);
        }
        if let Some(open) = self.pending_open.take().filter(|_| self.state == State::Opened) {
            open.store(true, Ordering::Release);
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.stop_capture();
        self.channel_id = None;
        self.state = State::WaitVersion;
        self.negotiated_formats.clear();
        debug!("AUDIO_INPUT channel closed");
    }
}

impl DvcClientProcessor for RdpeaiClient {}
