use core::fmt;

use ironrdp_core::{Decode as _, ReadCursor, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcMessage, SvcProcessor, SvcProcessorMessages, SvcServerProcessor};
use tracing::{debug, error, trace, warn};

use crate::pdu::{self, ClientAudioFormatPdu, QualityMode};

pub type RdpsndSvcMessages = SvcProcessorMessages<RdpsndServer>;

pub trait RdpsndError: core::error::Error + Send + Sync + 'static {}

impl<T> RdpsndError for T where T: core::error::Error + Send + Sync + 'static {}

/// Message sent by the event loop.
#[derive(Debug)]
pub enum RdpsndServerMessage {
    /// Wave data, with timestamp
    Wave(Vec<u8>, u32),
    SetVolume {
        left: u16,
        right: u16,
    },
    Close,
    /// Failure received from the OS event loop.
    ///
    /// Implementation should log/display this error.
    Error(Box<dyn RdpsndError>),
}

/// A server-offered audio format that the client also advertised support for,
/// paired with the `wFormatNo` the client expects for it on the wire.
///
/// The crate computes the set of these — the intersection of the server's
/// [`get_formats`] and the client's accepted formats — and hands it to
/// [`RdpsndServerHandler::choose_format`], which returns the one to stream.
///
/// `wformat_no` is intentionally private and there is no public constructor:
/// a handler can neither build nor mutate a `NegotiatedFormat`, so the index
/// stamped onto every Wave/Wave2 PDU is always a valid position in the
/// client's own format list. This makes it impossible to emit an out-of-range
/// `wFormatNo` (which a compliant client rejects, silently dropping all audio
/// — the classic footgun of the old index-returning API).
///
/// [`get_formats`]: RdpsndServerHandler::get_formats
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedFormat {
    /// The negotiated audio format (common to server and client).
    format: pdu::AudioFormat,
    /// Position of `format` in the client's Client Audio Formats list — the
    /// `wFormatNo` the client resolves each wave against. Crate-owned.
    wformat_no: u16,
}

impl NegotiatedFormat {
    /// The negotiated audio format — common to both server and client, and the
    /// one the returned wave data should match.
    pub fn format(&self) -> &pdu::AudioFormat {
        &self.format
    }

    /// Test-only accessor for the crate-private `wformat_no`, exposed for the
    /// integration testsuite behind the private `__test` feature. Not a stable API.
    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub fn wformat_no(&self) -> u16 {
        self.wformat_no
    }
}

/// Log rendering of one format: tag, rate, channels, bits and block alignment.
struct FormatSummary<'a>(&'a pdu::AudioFormat);

impl fmt::Display for FormatSummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let format = self.0;
        write!(
            f,
            "{} {}Hz {}ch {}bit align={}",
            format.format, format.n_samples_per_sec, format.n_channels, format.bits_per_sample, format.n_block_align
        )
    }
}

/// Log rendering of a format list. Each entry carries its position, which for
/// the client's list is the `wFormatNo` waves are sent under.
struct FormatList<'a>(&'a [pdu::AudioFormat]);

impl fmt::Display for FormatList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, format) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            write!(f, "[{index}] {}", FormatSummary(format))?;
        }
        Ok(())
    }
}

/// Where the wave last sent under a block number stands.
///
/// MS-RDPEA 2.2.3.8 describes one Wave Confirm per wave, sent once the data
/// is both received and played. Windows clients and xfreerdp3 both send two:
/// one on receipt that echoes the wave's timestamp, and a later one for the
/// same block whose timestamp is further on. Both are tracked so the second
/// is recognized, and reports the hold time, instead of reading as unmatched.
#[derive(Clone, Copy)]
enum BlockState {
    Free,
    /// Sent with this `wTimeStamp`, no confirm yet.
    Sent(u16),
    /// Sent with this `wTimeStamp` and confirmed once.
    Confirmed(u16),
}

/// What a Wave Confirm turned out to be.
#[cfg_attr(feature = "__test", visibility::make(pub))]
#[derive(Debug, PartialEq)]
enum ConfirmKind {
    /// The first confirm for a sent wave, carrying its send timestamp.
    First(u16),
    /// The second confirm for a wave, carrying its send timestamp.
    Second(u16),
    /// No wave under this block number is awaiting a confirm.
    Unmatched,
    /// A confirm for an earlier wave whose block number has since been
    /// reused: Its timestamp precedes the send timestamp of the wave now
    /// under that number.
    Stale,
}

/// Per block number, the wave last sent under it and how far it has been
/// confirmed.
///
/// The block number is only eight bits, so a slot is reused every 256 waves.
/// A confirm's `wTimeStamp` is the wave's own timestamp plus the time the
/// client held it (MS-RDPEA 3.2.5.2.1.6), so a confirm whose timestamp
/// precedes the slot's send timestamp cannot belong to the wave now in that
/// slot and is reported as stale. A late confirm whose timestamp happens to
/// fall after the newer send cannot be told apart from that wave's own, so
/// the classification is exact only while fewer than 256 waves are sent
/// within the client's confirm delay.
#[cfg_attr(feature = "__test", visibility::make(pub))]
struct SentBlocks([BlockState; 256]);

impl Default for SentBlocks {
    fn default() -> Self {
        Self([BlockState::Free; 256])
    }
}

impl SentBlocks {
    /// Records a wave sent under `block_no`, returning the timestamp of an
    /// earlier wave under the same number that was never confirmed at all.
    /// One confirmed once is fine: A client may send only the one confirm
    /// the specification describes.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    fn record(&mut self, block_no: u8, timestamp: u16) -> Option<u16> {
        match core::mem::replace(&mut self.0[usize::from(block_no)], BlockState::Sent(timestamp)) {
            BlockState::Sent(unconfirmed) => Some(unconfirmed),
            BlockState::Free | BlockState::Confirmed(_) => None,
        }
    }

    /// Classifies a Wave Confirm for `block_no` carrying `timestamp`.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    fn confirm(&mut self, block_no: u8, timestamp: u16) -> ConfirmKind {
        let slot = &mut self.0[usize::from(block_no)];
        match *slot {
            // A negative wrapping difference: the confirm predates this send.
            BlockState::Sent(sent) | BlockState::Confirmed(sent) if timestamp.wrapping_sub(sent) & 0x8000 != 0 => {
                ConfirmKind::Stale
            }
            BlockState::Sent(timestamp) => {
                *slot = BlockState::Confirmed(timestamp);
                ConfirmKind::First(timestamp)
            }
            BlockState::Confirmed(timestamp) => {
                *slot = BlockState::Free;
                ConfirmKind::Second(timestamp)
            }
            BlockState::Free => ConfirmKind::Unmatched,
        }
    }
}

impl fmt::Debug for SentBlocks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SentBlocks")
            .field(
                "unconfirmed",
                &self
                    .0
                    .iter()
                    .filter(|state| matches!(state, BlockState::Sent(_)))
                    .count(),
            )
            .finish()
    }
}

/// Stream counters, logged when the stream ends.
#[cfg_attr(feature = "__test", visibility::make(pub))]
#[derive(Debug, Default)]
struct StreamStats {
    pub waves_sent: u64,
    pub bytes_sent: u64,
    pub confirms: u64,
    /// Confirms that were the second one for their wave (see [`BlockState`]).
    pub second_confirms: u64,
    /// Confirms for a block with no wave awaiting one: a third confirm, or a
    /// block the server never sent.
    pub unmatched_confirms: u64,
    /// Waves whose block number came round again before any confirm arrived.
    pub never_confirmed: u64,
    /// Confirms for an earlier wave whose block number had already been reused.
    pub stale_confirms: u64,
}

/// Handler for the server side of the Audio Output Virtual Channel (`RDPSND`).
///
/// Implementations supply the list of audio formats the server offers, choose
/// which negotiated format to use once the client replies, and produce the
/// audio waves to stream (via [`RdpsndServer::wave`]).
pub trait RdpsndServerHandler: Send + fmt::Debug {
    /// The audio formats the server advertises in the Server Audio Formats and
    /// Version PDU (MS-RDPEA 2.2.2.1).
    fn get_formats(&self) -> &[pdu::AudioFormat];

    /// Select which format to stream, once the client has replied with the
    /// formats it accepts.
    ///
    /// `common` is the set of formats from [`get_formats`] that the client also
    /// advertised, in the server's preference order; each carries the
    /// `wFormatNo` the client expects, so the crate — not the handler — owns
    /// the index arithmetic and the MS-RDPEA rule that `wFormatNo` addresses
    /// the *client's* list. `common` is never empty: when server and client
    /// share no format, this method is not called and no audio is streamed.
    ///
    /// Return the [`NegotiatedFormat`] to stream (a reference borrowed from
    /// `common`), or [`None`] to decline. Returning a borrow from `common`
    /// — rather than an index or a constructed value — makes it impossible to
    /// pick a format the client did not accept or to produce an invalid
    /// `wFormatNo`. This is a pure selection step: any encoder/producer setup
    /// belongs in [`start`], which the crate calls next with the chosen format.
    ///
    /// [`get_formats`]: RdpsndServerHandler::get_formats
    /// [`start`]: RdpsndServerHandler::start
    fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat>;

    /// Begin streaming with the `format` just selected by [`choose_format`].
    ///
    /// Called once per session, immediately after a successful
    /// [`choose_format`]. This is the lifecycle hook: initialize encoder state,
    /// spawn the producer, etc. Waves are then emitted via [`RdpsndServer::wave`].
    ///
    /// Return `Err` if initialization fails (e.g. the encoder can't be created).
    /// The crate then **declines the negotiated format** — exactly as if
    /// [`choose_format`] had returned [`None`] — rather than leaving the channel
    /// "negotiated" but silently producing no audio. The error is logged by the
    /// crate.
    ///
    /// [`choose_format`]: RdpsndServerHandler::choose_format
    fn start(&mut self, format: &NegotiatedFormat) -> Result<(), Box<dyn RdpsndError>>;

    /// Called when the audio stream is torn down (e.g. the client closed the
    /// channel or the session ended).
    fn stop(&mut self);

    /// Called for every Wave Confirm PDU the client sends.
    ///
    /// `timestamp` answers the `wTimeStamp` the server put on the wave with
    /// this `block_no`. It is not an echo: [\[MS-RDPEA\] 2.2.3.8] sets it to
    /// that value *plus* the milliseconds between receiving the complete wave
    /// and sending the confirm, so the difference from the wave's own
    /// timestamp is the time the client held the data, not a round trip.
    ///
    /// One block may be confirmed more than once, with different timestamps.
    /// The default implementation ignores it.
    ///
    /// [\[MS-RDPEA\] 2.2.3.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/1c67d6d0-4e8b-4e1a-9d3a-cd0d6f0d1c5f
    fn wave_confirm(&mut self, block_no: u8, timestamp: u16) {
        let _ = (block_no, timestamp);
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RdpsndState {
    Start,
    WaitingForClientFormats,
    WaitingForQualityMode,
    WaitingForTrainingConfirm,
    Ready,
    Stop,
}

#[derive(Debug)]
pub struct RdpsndServer {
    handler: Box<dyn RdpsndServerHandler>,
    state: RdpsndState,
    client_format: Option<ClientAudioFormatPdu>,
    quality_mode: Option<QualityMode>,
    block_no: u8,
    format_no: Option<u16>,
    sent_blocks: SentBlocks,
    stats: StreamStats,
}

impl RdpsndServer {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

    pub fn new(handler: Box<dyn RdpsndServerHandler>) -> Self {
        Self {
            handler,
            state: RdpsndState::Start,
            client_format: None,
            quality_mode: None,
            format_no: None,
            block_no: 0,
            sent_blocks: SentBlocks::default(),
            stats: StreamStats::default(),
        }
    }

    pub fn version(&self) -> PduResult<pdu::Version> {
        let client_format = self
            .client_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state, client format not yet received"))?;

        Ok(client_format.version)
    }

    /// Whether negotiation has finished and a format was accepted, so wave data
    /// has somewhere to go.
    pub fn is_ready(&self) -> bool {
        self.state == RdpsndState::Ready && self.format_no.is_some()
    }

    #[cfg(feature = "__test")]
    pub fn stats(&self) -> &StreamStats {
        &self.stats
    }

    pub fn flags(&self) -> PduResult<pdu::AudioFormatFlags> {
        let client_format = self
            .client_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state, client format not yet received"))?;

        Ok(client_format.flags)
    }

    pub fn training_pdu(&mut self) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::TrainingPdu {
            timestamp: 4231, // a random number
            data: vec![],
        };
        debug!(timestamp = pdu.timestamp, "Sending RDPSND training PDU");
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ServerAudioOutputPdu::Training(pdu).into(),
        ]))
    }

    pub fn wave(&mut self, data: Vec<u8>, ts: u32) -> PduResult<RdpsndSvcMessages> {
        let version = self.version()?;
        let format_no = self
            .format_no
            .ok_or_else(|| pdu_other_err!("invalid state - no format"))?;

        // The client answers wTimeStamp in the Wave Confirm PDU, offset by how
        // long it held the data, so sending zero leaves that measurement no
        // reference to be relative to. Carry the low bits of the same capture
        // time the 32-bit dwAudioTimeStamp gets, so both fields describe one
        // instant.
        let [timestamp_lo, timestamp_hi, _, _] = ts.to_le_bytes();
        let wire_timestamp = u16::from_le_bytes([timestamp_lo, timestamp_hi]);
        let data_len = data.len();

        // The server doesn't wait for wave confirm, apparently FreeRDP neither.
        let msg = if version >= pdu::Version::V8 {
            let pdu = pdu::Wave2Pdu {
                block_no: self.block_no,
                timestamp: wire_timestamp,
                audio_timestamp: ts,
                format_no,
                data: data.into(),
            };
            RdpsndSvcMessages::new(vec![pdu::ServerAudioOutputPdu::Wave2(pdu).into()])
        } else {
            // Pre-v8: WaveInfo PDU (§2.2.3.3), then a bare Wave payload (§2.2.3.4).
            if data.len() < usize::from(pdu::WavePdu::MIN_AUDIO_LENGTH) {
                return Err(pdu_other_err!("wave data shorter than WaveInfo Data prefix"));
            }
            // BodySize = 8 + audio_length must fit in the 16-bit RDPSND header field.
            if data.len() > usize::from(pdu::WavePdu::MAX_AUDIO_LENGTH) {
                return Err(pdu_other_err!("wave data too large for WaveInfo BodySize"));
            }
            let audio_length = u16::try_from(data.len()).map_err(|_| pdu_other_err!("wave data too large"))?;
            let mut data_prefix = [0u8; 4];
            data_prefix.copy_from_slice(&data[..4]);
            let info = pdu::WavePdu {
                block_no: self.block_no,
                format_no,
                timestamp: wire_timestamp,
                data_prefix,
                audio_length,
            };
            let wave_data = pdu::WaveDataPdu {
                data: data[4..].to_vec(),
            };
            RdpsndSvcMessages::new(vec![pdu::ServerAudioOutputPdu::Wave(info).into(), wave_data.into()])
        };

        if let Some(sent_timestamp) = self.sent_blocks.record(self.block_no, wire_timestamp) {
            self.stats.never_confirmed += 1;
            trace!(
                block_no = self.block_no,
                sent_timestamp, "Reusing the block number of a wave the client never confirmed"
            );
        }
        self.stats.waves_sent += 1;
        self.stats.bytes_sent += u64::try_from(data_len).unwrap_or(u64::MAX);
        trace!(
            block_no = self.block_no,
            format_no,
            audio_timestamp = ts,
            len = data_len,
            "Sending RDPSND wave"
        );

        self.block_no = self.block_no.overflowing_add(1).0;

        Ok(msg)
    }

    pub fn set_volume(&mut self, volume_left: u16, volume_right: u16) -> PduResult<RdpsndSvcMessages> {
        if !self.flags()?.contains(pdu::AudioFormatFlags::VOLUME) {
            return Err(pdu_other_err!("client doesn't support volume"));
        }
        let pdu = pdu::VolumePdu {
            volume_left,
            volume_right,
        };
        debug!(volume_left, volume_right, "Sending RDPSND volume");
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ServerAudioOutputPdu::Volume(pdu).into(),
        ]))
    }

    pub fn close(&mut self) -> PduResult<RdpsndSvcMessages> {
        debug!("Sending RDPSND close");
        Ok(RdpsndSvcMessages::new(vec![pdu::ServerAudioOutputPdu::Close.into()]))
    }
}

/// Build the set of formats common to the server (`server_formats`, kept in the
/// server's preference order) and the client (`client_formats`), each tagged
/// with its `wFormatNo` — its index in the *client's* list, which is what the
/// client resolves waves against (MS-RDPEA). The result mirrors the server's
/// ordering so the handler can express preference simply by `get_formats`
/// order, while the `wFormatNo` always points into the client list.
#[cfg_attr(feature = "__test", visibility::make(pub))]
fn negotiate_formats(
    server_formats: &[pdu::AudioFormat],
    client_formats: &[pdu::AudioFormat],
) -> Vec<NegotiatedFormat> {
    server_formats
        .iter()
        .filter_map(|server_format| {
            client_formats
                .iter()
                .position(|client_fmt| client_fmt.matches_for_negotiation(server_format))
                .and_then(|idx| u16::try_from(idx).ok())
                .map(|wformat_no| NegotiatedFormat {
                    format: server_format.clone(),
                    wformat_no,
                })
        })
        .collect()
}

impl_as_any!(RdpsndServer);

impl SvcProcessor for RdpsndServer {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = pdu::ClientAudioOutputPdu::decode(&mut ReadCursor::new(payload)).map_err(|e| decode_err!(e))?;
        let msg = match self.state {
            RdpsndState::WaitingForClientFormats => {
                let pdu::ClientAudioOutputPdu::AudioFormat(af) = pdu else {
                    error!(?pdu, "Expected the client audio formats; stopping the RDPSND channel");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                debug!(
                    version = ?af.version,
                    flags = ?af.flags,
                    volume_left = af.volume_left,
                    volume_right = af.volume_right,
                    pitch = af.pitch,
                    dgram_port = af.dgram_port,
                    count = af.formats.len(),
                    formats = %FormatList(&af.formats),
                    "Received client audio formats"
                );
                // MS-RDPEA 2.2.2.2: without TSSNDCAPS_ALIVE the client does not
                // consume audio, whatever format ends up negotiated.
                if !af.flags.contains(pdu::AudioFormatFlags::ALIVE) {
                    warn!(flags = ?af.flags, "Client audio formats lack the ALIVE flag; the client may play nothing");
                }
                self.client_format = Some(af);
                if self.version()? >= pdu::Version::V6 {
                    self.state = RdpsndState::WaitingForQualityMode;
                    vec![]
                } else {
                    self.state = RdpsndState::WaitingForTrainingConfirm;
                    self.training_pdu()?.into()
                }
            }
            RdpsndState::WaitingForQualityMode => {
                let pdu::ClientAudioOutputPdu::QualityMode(pdu) = pdu else {
                    error!(?pdu, "Expected the client quality mode; stopping the RDPSND channel");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                debug!(quality_mode = ?pdu.quality_mode, "Received client quality mode");
                self.quality_mode = Some(pdu.quality_mode);
                self.state = RdpsndState::WaitingForTrainingConfirm;
                self.training_pdu()?.into()
            }
            RdpsndState::WaitingForTrainingConfirm => {
                let pdu::ClientAudioOutputPdu::TrainingConfirm(training_confirm) = pdu else {
                    error!(
                        ?pdu,
                        "Expected the client training confirm; stopping the RDPSND channel"
                    );
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                debug!(
                    timestamp = training_confirm.timestamp,
                    pack_size = training_confirm.pack_size,
                    "Received RDPSND training confirm"
                );
                let client_format = self.client_format.as_ref().expect("available in this state");
                // Formats common to server and client, in the server's
                // preference order, each tagged with its wFormatNo (its
                // position in the *client's* list). Keeping this in the crate
                // means the handler never does index arithmetic and can't emit
                // an out-of-range wFormatNo.
                let common = negotiate_formats(self.handler.get_formats(), &client_format.formats);
                self.state = RdpsndState::Ready;
                if common.is_empty() {
                    debug!(
                        server_formats = self.handler.get_formats().len(),
                        client_formats = client_format.formats.len(),
                        "No audio format in common with the client; audio disabled"
                    );
                } else if let Some(chosen) = self.handler.choose_format(&common) {
                    // `chosen` borrows `common` (a local), not `self`, so the
                    // handler is free to borrow `&mut self` again for `start`.
                    let wformat_no = chosen.wformat_no;
                    debug!(
                        wformat_no,
                        format = %FormatSummary(chosen.format()),
                        common = common.len(),
                        "Audio format chosen"
                    );
                    // Commit the index BEFORE the `start` lifecycle hook: if `start`
                    // spawns a producer that emits a wave immediately, `wave()` must
                    // already see a valid `format_no` rather than racing an unset one.
                    self.format_no = Some(wformat_no);
                    if let Err(e) = self.handler.start(chosen) {
                        // Initialization failed (e.g. the encoder couldn't be
                        // created). Roll back to a cleanly *declined* state — the
                        // same outcome as `choose_format` returning `None` — instead
                        // of leaving the channel "negotiated" but silently producing
                        // no audio.
                        error!(error = %e, "rdpsnd handler failed to start; declining the negotiated format");
                        self.format_no = None;
                    }
                } else {
                    debug!(
                        common = common.len(),
                        "Handler declined every common audio format; audio disabled"
                    );
                }
                vec![]
            }
            RdpsndState::Ready => {
                if let pdu::ClientAudioOutputPdu::WaveConfirm(c) = pdu {
                    self.stats.confirms += 1;
                    match self.sent_blocks.confirm(c.block_no, c.timestamp) {
                        ConfirmKind::First(sent_timestamp) => trace!(
                            block_no = c.block_no,
                            timestamp = c.timestamp,
                            held_ms = c.timestamp.wrapping_sub(sent_timestamp),
                            "Received RDPSND wave confirm"
                        ),
                        ConfirmKind::Second(sent_timestamp) => {
                            self.stats.second_confirms += 1;
                            trace!(
                                block_no = c.block_no,
                                timestamp = c.timestamp,
                                held_ms = c.timestamp.wrapping_sub(sent_timestamp),
                                "Received second RDPSND wave confirm"
                            );
                        }
                        ConfirmKind::Stale => {
                            self.stats.stale_confirms += 1;
                            trace!(
                                block_no = c.block_no,
                                timestamp = c.timestamp,
                                "Received RDPSND wave confirm for a wave whose block number was already reused"
                            );
                        }
                        ConfirmKind::Unmatched => {
                            self.stats.unmatched_confirms += 1;
                            trace!(
                                block_no = c.block_no,
                                timestamp = c.timestamp,
                                "Received RDPSND wave confirm for a block with no wave awaiting one"
                            );
                        }
                    }
                    self.handler.wave_confirm(c.block_no, c.timestamp);
                } else {
                    debug!(?pdu, "Ignoring RDPSND PDU received after negotiation");
                }
                vec![]
            }
            state => {
                error!(?state, ?pdu, "Received an RDPSND PDU in a state that expects none");
                vec![]
            }
        };
        Ok(msg)
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.state != RdpsndState::Start {
            error!(state = ?self.state, "Attempted to start rdpsnd channel in invalid state");
        }

        let formats = self.handler.get_formats();
        debug!(
            count = formats.len(),
            formats = %FormatList(formats),
            "Sending server audio formats"
        );
        let pdu = pdu::ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
            version: pdu::Version::V8,
            formats: formats.into(),
        });

        self.state = RdpsndState::WaitingForClientFormats;
        Ok(vec![SvcMessage::from(pdu)])
    }
}

impl Drop for RdpsndServer {
    fn drop(&mut self) {
        if self.stats.waves_sent > 0 {
            debug!(stats = ?self.stats, "RDPSND stream ended");
        }
        self.handler.stop();
    }
}

impl SvcServerProcessor for RdpsndServer {}
