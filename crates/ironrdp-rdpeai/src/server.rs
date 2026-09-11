//! AUDIO_INPUT dynamic virtual channel server (MS-RDPEAI).

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use tracing::{debug, trace, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{DataPdu, FormatChangePdu, FormatsPdu, OpenPdu, OpenReplyPdu, RdpeaiPdu, Version, VersionPdu};

pub trait RdpeaiError: core::error::Error + Send + Sync + 'static {}

impl<T> RdpeaiError for T where T: core::error::Error + Send + Sync + 'static {}

/// Message sent by the embedding application's event loop to drive [`RdpeaiServer`] from
/// outside the DVC message-processing path — e.g. once a consumer decides it wants to start
/// recording, or wants to switch formats mid-session.
#[derive(Debug)]
pub enum RdpeaiServerMessage {
    /// Request the client start recording. See [`RdpeaiServer::open`].
    Open {
        frames_per_packet: u32,
        initial_format: u32,
        capture_format: AudioFormat,
    },
    /// Request the client switch to a different negotiated format. See
    /// [`RdpeaiServer::change_format`].
    ChangeFormat { new_format: u32 },
    /// Failure received from the embedding application's own capture/consumer pipeline.
    /// Implementations should log/display this error.
    Error(Box<dyn RdpeaiError>),
}

/// Handler for the server side of the Audio Input Redirection Virtual Channel (`AUDIO_INPUT`).
///
/// Implementations supply the list of audio formats the server offers and receive the
/// client's captured audio once [`RdpeaiServer::open`] succeeds. `RdpeaiServer` owns the
/// MS-RDPEAI state machine; the backend only reacts to negotiated state and produces raw
/// audio bytes to consumers (e.g. a PipeWire virtual microphone source).
pub trait RdpeaiServerBackend: Send {
    /// Formats this server offers, in preferred order. Sent verbatim in the Sound Formats PDU
    /// (MS-RDPEAI 2.2.2.2) once the client's Version PDU is processed.
    fn supported_formats(&self) -> &[AudioFormat];

    /// Called once the client has replied with its own Sound Formats PDU, establishing the
    /// final negotiated list (MS-RDPEAI 3.3.5.1.5). `negotiated` is the client's subset, in the
    /// client's order — the same list [`RdpeaiServer::open`]'s `initial_format` and
    /// [`RdpeaiServer::change_format`]'s `new_format` index into. May be empty if the client
    /// advertised no format the server also offers, in which case recording is unavailable
    /// for the session.
    fn on_formats_negotiated(&mut self, negotiated: &[AudioFormat]) {
        let _ = negotiated;
    }

    /// Called when the client answers an Open PDU (MS-RDPEAI 3.3.5.1.8). `result` is the
    /// client's raw HRESULT; per MS-RDPEAI 3.3.5.1.8 an HRESULT is an error only when its
    /// sign bit is set, so `result >= 0` (not just [`OpenReplyPdu::S_OK`]) means capture
    /// started.
    fn on_open_reply(&mut self, result: i32) {
        let _ = result;
    }

    /// Called for every decoded audio packet the client sends while opened
    /// (MS-RDPEAI 3.3.5.2.2). `format` is the format currently in effect. `data` is the raw
    /// payload from the Data PDU, encoded per `format` — this crate does not decode compressed
    /// formats; PCM formats are already raw samples.
    fn on_audio_data(&mut self, format: &AudioFormat, data: &[u8]);

    /// Called once the client confirms a server-initiated format change
    /// (MS-RDPEAI 3.3.5.3.2). Not called for the initial format confirm that precedes the
    /// first Open Reply — see [`Self::on_open_reply`] for that.
    fn on_format_change_confirmed(&mut self, format: &AudioFormat) {
        let _ = format;
    }

    /// Called when the dynamic virtual channel is torn down.
    fn on_close(&mut self) {}
}

/// A [`RdpeaiServerBackend`] that offers no formats and drops all audio.
#[derive(Debug, Default)]
pub struct NoopRdpeaiServerBackend;

impl RdpeaiServerBackend for NoopRdpeaiServerBackend {
    fn supported_formats(&self) -> &[AudioFormat] {
        &[]
    }

    fn on_audio_data(&mut self, _format: &AudioFormat, _data: &[u8]) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Channel not yet started.
    Start,
    /// Server Version PDU sent; waiting for the client's Version PDU.
    AwaitingClientVersion,
    /// Server Formats PDU sent; waiting for the client's Formats PDU.
    AwaitingClientFormats,
    /// Formats negotiated; no Open outstanding.
    Ready,
    /// Open PDU sent; waiting for the client's FormatChange confirming the initial format.
    AwaitingFormatConfirm,
    /// Initial FormatChange confirmed; waiting for the client's Open Reply.
    AwaitingOpenReply,
    /// Capture device open; Data PDUs expected.
    Opened,
}

/// Server processor for the `AUDIO_INPUT` dynamic virtual channel (MS-RDPEAI).
///
/// Drives the initialization sequence automatically on channel start (Version, then Formats,
/// per 3.3.5.1.1-3.3.5.1.5): the server always advertises its formats immediately, since
/// nothing in the protocol depends on a higher-layer "start recording" decision until
/// [`Self::open`] is called. Recording itself is caller-initiated: call [`Self::open`] once
/// the negotiated formats are known ([`RdpeaiServerBackend::on_formats_negotiated`]) and the
/// consuming application actually wants to record, then forward the returned PDUs to the
/// client over the channel id this processor was created for (recover it with
/// [`ironrdp_dvc::DrdynvcServer::get_channel_id_by_type`]).
///
/// Malformed, unrecognized, or out-of-sequence PDUs are logged and ignored rather than
/// treated as errors, per MS-RDPEAI 3.1.5's explicit MUST-ignore requirement.
pub struct RdpeaiServer {
    backend: Box<dyn RdpeaiServerBackend>,
    state: State,
    client_version: Option<Version>,
    /// Negotiated format list, in the client's reply order — Open/FormatChange indices refer
    /// here (MS-RDPEAI 3.1.1: this is the list the client establishes as final).
    negotiated_formats: Vec<AudioFormat>,
    /// Index into `negotiated_formats` currently in effect once opened.
    current_format: Option<u32>,
    /// `initialFormat` from the outstanding Open PDU, pending the client's confirming
    /// FormatChange.
    pending_open_format: Option<u32>,
    /// `NewFormat` from an outstanding server-initiated FormatChange, pending confirm.
    pending_format_change: Option<u32>,
}

impl RdpeaiServer {
    pub fn new(backend: Box<dyn RdpeaiServerBackend>) -> Self {
        Self {
            backend,
            state: State::Start,
            client_version: None,
            negotiated_formats: Vec::new(),
            current_format: None,
            pending_open_format: None,
            pending_format_change: None,
        }
    }

    /// Clear all per-connection state, transitioning to `state`. Shared by
    /// [`DvcProcessor::start`] (transitions to `AwaitingClientVersion`) and
    /// [`DvcProcessor::close`] (transitions to `Start`), so the two reset sequences can't
    /// drift apart as fields are added.
    fn reset(&mut self, state: State) {
        self.state = state;
        self.client_version = None;
        self.negotiated_formats.clear();
        self.current_format = None;
        self.pending_open_format = None;
        self.pending_format_change = None;
    }

    /// Formats negotiated with the client (MS-RDPEAI 3.3.5.1.5); empty before that completes
    /// or if no common format exists.
    pub fn negotiated_formats(&self) -> &[AudioFormat] {
        &self.negotiated_formats
    }

    /// The format currently in effect, once [`Self::open`] has succeeded.
    pub fn current_format(&self) -> Option<&AudioFormat> {
        self.current_format.and_then(|idx| self.resolve_format(idx))
    }

    /// The protocol version the client advertised, once its Version PDU has been processed.
    pub fn client_version(&self) -> Option<Version> {
        self.client_version
    }

    fn resolve_format(&self, index: u32) -> Option<&AudioFormat> {
        let idx = usize::try_from(index).ok()?;
        self.negotiated_formats.get(idx)
    }

    /// Request the client start recording (MS-RDPEAI 3.3.5.1.6 / 3.1.4.1).
    ///
    /// `initial_format` indexes [`Self::negotiated_formats`] and selects the encoding used for
    /// captured Data PDUs; `capture_format` is the WAVEFORMATEX the client's capture device is
    /// asked to open with (MS-RDPEAI 2.2.2.3) and MAY differ from the encoding format when the
    /// client captures PCM and encodes afterward. `frames_per_packet` bounds each Data PDU to
    /// `nChannels * 2 * FramesPerPacket` bytes (2.2.2.3 / 2.2.3.2).
    ///
    /// Valid only once formats are negotiated and no Open is already outstanding or active;
    /// returns an error otherwise rather than sending a second, contract-violating Open PDU.
    /// Errors are returned, not panics, since the caller may race the negotiation completing.
    pub fn open(
        &mut self,
        frames_per_packet: u32,
        initial_format: u32,
        capture_format: AudioFormat,
    ) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::Ready {
            return Err(pdu_other_err!("RdpeaiServer::open called outside the Ready state"));
        }
        if self.resolve_format(initial_format).is_none() {
            return Err(pdu_other_err!("RdpeaiServer::open initial_format out of range"));
        }

        self.pending_open_format = Some(initial_format);
        self.state = State::AwaitingFormatConfirm;

        Ok(vec![Box::new(RdpeaiPdu::Open(OpenPdu {
            frames_per_packet,
            initial_format,
            capture_format,
        }))])
    }

    /// Request the client switch to a different negotiated format while streaming
    /// (MS-RDPEAI 3.3.5.3.1). `new_format` indexes [`Self::negotiated_formats`].
    ///
    /// Per 3.3.5.3.1, when the currently active format is AAC and the client only advertised
    /// protocol version 1, the server SHOULD NOT send this PDU: that case logs a warning and
    /// returns no messages rather than sending a PDU the spec discourages. Valid only while
    /// [`Self::current_format`] is set (i.e. after a successful [`Self::open`]).
    pub fn change_format(&mut self, new_format: u32) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::Opened {
            return Err(pdu_other_err!(
                "RdpeaiServer::change_format called outside the Opened state"
            ));
        }
        if self.resolve_format(new_format).is_none() {
            return Err(pdu_other_err!("RdpeaiServer::change_format new_format out of range"));
        }

        let active_is_aac = self
            .current_format()
            .is_some_and(|fmt| fmt.format == WaveFormat::AAC_MS);
        if active_is_aac && self.client_version == Some(Version::V1) {
            warn!(
                "Skipping AUDIO_INPUT FormatChange: active format is AAC and the client only \
                 advertised protocol version 1 (MS-RDPEAI 3.3.5.3.1 SHOULD NOT)"
            );
            return Ok(Vec::new());
        }

        self.pending_format_change = Some(new_format);
        Ok(vec![Box::new(RdpeaiPdu::FormatChange(FormatChangePdu::new(
            new_format,
        )))])
    }

    fn handle_version(&mut self, pdu: VersionPdu) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::AwaitingClientVersion {
            warn!(?self.state, "Ignoring out-of-sequence AUDIO_INPUT Version PDU");
            return Ok(Vec::new());
        }
        self.client_version = Some(pdu.version);
        self.state = State::AwaitingClientFormats;
        debug!(client_version = ?pdu.version, "AUDIO_INPUT version received");
        // MS-RDPEAI 3.3.5.1.3: the server MUST send a Sound Formats PDU after processing the
        // client's Version PDU.
        Ok(vec![Box::new(RdpeaiPdu::Formats(FormatsPdu::server(
            self.backend.supported_formats().to_vec(),
        )))])
    }

    fn handle_formats(&mut self, client: FormatsPdu) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::AwaitingClientFormats {
            warn!(?self.state, "Ignoring out-of-sequence AUDIO_INPUT Formats PDU");
            return Ok(Vec::new());
        }

        // MS-RDPEAI 3.2.5.1.5: the client's list MUST be a subset of what the server offered.
        // Enforce defensively rather than trusting the client's claim verbatim.
        let offered = self.backend.supported_formats();
        let negotiated: Vec<AudioFormat> = client
            .formats
            .into_iter()
            .filter(|fmt| offered.iter().any(|server_fmt| fmt.matches_for_negotiation(server_fmt)))
            .collect();

        if negotiated.is_empty() {
            warn!("No common AUDIO_INPUT formats after client reply; recording is unavailable this session");
        } else {
            debug!(count = negotiated.len(), "AUDIO_INPUT formats negotiated");
        }

        self.negotiated_formats = negotiated;
        self.state = State::Ready;
        self.backend.on_formats_negotiated(&self.negotiated_formats);
        Ok(Vec::new())
    }

    fn handle_format_change(&mut self, pdu: FormatChangePdu) -> PduResult<Vec<DvcMessage>> {
        match self.state {
            State::AwaitingFormatConfirm => {
                // MS-RDPEAI 3.3.5.1.7: before the Open Reply, the client confirms the initial
                // format chosen by the server's Open PDU.
                let Some(pending) = self.pending_open_format else {
                    warn!("AwaitingFormatConfirm with no pending Open format; ignoring");
                    return Ok(Vec::new());
                };
                if pdu.new_format != pending {
                    // MS-RDPEAI 3.1.5: a non-conformant confirm MUST be ignored, not trusted.
                    // Keep the server-requested index rather than the client's divergent echo.
                    warn!(
                        expected = pending,
                        got = pdu.new_format,
                        "Initial AUDIO_INPUT FormatChange confirm does not match the format from Open; \
                         keeping the requested format"
                    );
                }
                self.current_format = Some(pending);
                self.pending_open_format = None;
                self.state = State::AwaitingOpenReply;
                Ok(Vec::new())
            }
            State::Opened => {
                // MS-RDPEAI 3.3.5.3.2: client confirms a server-initiated format change.
                let Some(pending) = self.pending_format_change.take() else {
                    warn!("Unsolicited AUDIO_INPUT FormatChange confirm while Opened; ignoring");
                    return Ok(Vec::new());
                };
                if pdu.new_format != pending {
                    // MS-RDPEAI 3.1.5: a non-conformant confirm MUST be ignored, not trusted.
                    // Keep the server-requested index rather than the client's divergent echo.
                    warn!(
                        expected = pending,
                        got = pdu.new_format,
                        "AUDIO_INPUT FormatChange confirm does not match the requested format; \
                         keeping the requested format"
                    );
                }
                self.current_format = Some(pending);
                if let Some(fmt) = self.current_format().cloned() {
                    self.backend.on_format_change_confirmed(&fmt);
                }
                Ok(Vec::new())
            }
            _ => {
                warn!(?self.state, "Ignoring out-of-sequence AUDIO_INPUT FormatChange PDU");
                Ok(Vec::new())
            }
        }
    }

    fn handle_open_reply(&mut self, pdu: OpenReplyPdu) -> PduResult<Vec<DvcMessage>> {
        match self.state {
            State::AwaitingOpenReply => {
                self.backend.on_open_reply(pdu.result);
                // MS-RDPEAI 3.3.5.1.8: an HRESULT is an error only when its sign bit is set,
                // so any non-negative result (not just S_OK) is success.
                if pdu.result >= 0 {
                    self.state = State::Opened;
                    debug!("AUDIO_INPUT capture opened");
                } else {
                    // MS-RDPEAI 3.3.5.1.8: on failure the server MAY send another Open PDU;
                    // leave that to the caller by returning to Ready rather than retrying
                    // automatically.
                    self.current_format = None;
                    self.state = State::Ready;
                    warn!(
                        result = pdu.result,
                        "AUDIO_INPUT client failed to open its capture device"
                    );
                }
            }
            State::AwaitingFormatConfirm => {
                // A client rejecting Open before confirming the initial format (e.g.
                // initialFormat out of range, or FramesPerPacket rejected) skips the
                // FormatChange confirm and replies with OpenReply failure directly.
                // 3.3.5.1.8 only conditions the server's reaction on the Result field, not on
                // a preceding FormatChange, so accept it here too rather than leaving the
                // channel wedged in AwaitingFormatConfirm with no path back to Ready.
                self.backend.on_open_reply(pdu.result);
                self.pending_open_format = None;
                self.current_format = None;
                self.state = State::Ready;
                warn!(
                    result = pdu.result,
                    "AUDIO_INPUT client rejected Open before confirming the initial format"
                );
            }
            _ => {
                warn!(?self.state, "Ignoring out-of-sequence AUDIO_INPUT OpenReply PDU");
            }
        }
        Ok(Vec::new())
    }

    fn handle_data(&mut self, pdu: DataPdu) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::Opened {
            warn!(?self.state, "Ignoring AUDIO_INPUT Data PDU outside the Opened state");
            return Ok(Vec::new());
        }
        let Some(format) = self.current_format().cloned() else {
            warn!("Opened with no current format resolved; dropping audio data");
            return Ok(Vec::new());
        };
        self.backend.on_audio_data(&format, &pdu.data);
        Ok(Vec::new())
    }
}

impl_as_any!(RdpeaiServer);

impl DvcProcessor for RdpeaiServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.reset(State::AwaitingClientVersion);
        debug!(channel_id, "AUDIO_INPUT channel started");
        // MS-RDPEAI 3.3.5.1.1: the Version PDU MUST be the first PDU sent by the server.
        Ok(vec![Box::new(RdpeaiPdu::Version(VersionPdu::new(Version::V2)))])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        // MS-RDPEAI 3.1.5: malformed or unrecognized PDUs MUST be ignored rather than
        // treated as errors. A decode failure must not propagate: this processor is
        // reached through the shared DRDYNVC SVC processor, so an Err here would fail
        // the whole dynamic-channel message loop, not just this channel.
        let pdu: RdpeaiPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(e) => {
                warn!(error = %e, "Ignoring malformed AUDIO_INPUT PDU");
                return Ok(Vec::new());
            }
        };
        trace!(?pdu, "AUDIO_INPUT PDU received");
        match pdu {
            RdpeaiPdu::Version(v) => self.handle_version(v),
            RdpeaiPdu::Formats(f) => self.handle_formats(f),
            RdpeaiPdu::FormatChange(c) => self.handle_format_change(c),
            RdpeaiPdu::OpenReply(r) => self.handle_open_reply(r),
            // MS-RDPEAI 3.3.5.1.4 / 3.3.5.2.1: diagnostic only, precedes Formats or Data.
            RdpeaiPdu::DataIncoming => Ok(Vec::new()),
            RdpeaiPdu::Data(d) => self.handle_data(d),
            RdpeaiPdu::Open(_) => {
                warn!("Ignoring server-originated AUDIO_INPUT Open PDU received from the client");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.backend.on_close();
        self.reset(State::Start);
        debug!("AUDIO_INPUT channel closed");
    }
}

impl DvcServerProcessor for RdpeaiServer {}
