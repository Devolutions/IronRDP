use ironrdp_core::{Decode as _, ReadCursor, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcMessage, SvcProcessor, SvcProcessorMessages, SvcServerProcessor};
use tracing::{debug, error};

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
}

/// Handler for the server side of the Audio Output Virtual Channel (`RDPSND`).
///
/// Implementations supply the list of audio formats the server offers, choose
/// which negotiated format to use once the client replies, and produce the
/// audio waves to stream (via [`RdpsndServer::wave`]).
pub trait RdpsndServerHandler: Send + core::fmt::Debug {
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
    /// [`choose_format`]: RdpsndServerHandler::choose_format
    fn start(&mut self, format: &NegotiatedFormat);

    /// Called when the audio stream is torn down (e.g. the client closed the
    /// channel or the session ended).
    fn stop(&mut self);
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
        }
    }

    pub fn version(&self) -> PduResult<pdu::Version> {
        let client_format = self
            .client_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state, client format not yet received"))?;

        Ok(client_format.version)
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
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ServerAudioOutputPdu::Training(pdu).into(),
        ]))
    }

    pub fn wave(&mut self, data: Vec<u8>, ts: u32) -> PduResult<RdpsndSvcMessages> {
        let version = self.version()?;
        let format_no = self
            .format_no
            .ok_or_else(|| pdu_other_err!("invalid state - no format"))?;

        // The server doesn't wait for wave confirm, apparently FreeRDP neither.
        let msg = if version >= pdu::Version::V8 {
            let pdu = pdu::Wave2Pdu {
                block_no: self.block_no,
                timestamp: 0,
                audio_timestamp: ts,
                format_no,
                data: data.into(),
            };
            RdpsndSvcMessages::new(vec![pdu::ServerAudioOutputPdu::Wave2(pdu).into()])
        } else {
            let pdu = pdu::WavePdu {
                block_no: self.block_no,
                format_no,
                timestamp: 0,
                data: data.into(),
            };
            RdpsndSvcMessages::new(vec![pdu::ServerAudioOutputPdu::Wave(pdu).into()])
        };

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
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ServerAudioOutputPdu::Volume(pdu).into(),
        ]))
    }

    pub fn close(&mut self) -> PduResult<RdpsndSvcMessages> {
        Ok(RdpsndSvcMessages::new(vec![pdu::ServerAudioOutputPdu::Close.into()]))
    }
}

/// Build the set of formats common to the server (`server_formats`, kept in the
/// server's preference order) and the client (`client_formats`), each tagged
/// with its `wFormatNo` — its index in the *client's* list, which is what the
/// client resolves waves against (MS-RDPEA). The result mirrors the server's
/// ordering so the handler can express preference simply by `get_formats`
/// order, while the `wFormatNo` always points into the client list.
fn negotiate_formats(
    server_formats: &[pdu::AudioFormat],
    client_formats: &[pdu::AudioFormat],
) -> Vec<NegotiatedFormat> {
    server_formats
        .iter()
        .filter_map(|server_format| {
            client_formats
                .iter()
                .position(|client_fmt| audio_format_eq(client_fmt, server_format))
                .and_then(|idx| u16::try_from(idx).ok())
                .map(|wformat_no| NegotiatedFormat {
                    format: server_format.clone(),
                    wformat_no,
                })
        })
        .collect()
}

/// Compare two audio formats by their WAVEFORMATEX identity — wave format tag,
/// channel count, sample rate, and bit depth. Derived fields
/// (`n_avg_bytes_per_sec`, `n_block_align`) and the codec-specific `data` blob
/// are deliberately ignored: a client echoes back a format it accepts but is
/// not guaranteed to reproduce those byte-for-byte.
fn audio_format_eq(a: &pdu::AudioFormat, b: &pdu::AudioFormat) -> bool {
    a.format == b.format
        && a.n_channels == b.n_channels
        && a.n_samples_per_sec == b.n_samples_per_sec
        && a.bits_per_sample == b.bits_per_sample
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
        debug!(?pdu);
        let msg = match self.state {
            RdpsndState::WaitingForClientFormats => {
                let pdu::ClientAudioOutputPdu::AudioFormat(af) = pdu else {
                    error!("Invalid PDU");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
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
                    error!("Invalid PDU");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                self.quality_mode = Some(pdu.quality_mode);
                self.state = RdpsndState::WaitingForTrainingConfirm;
                self.training_pdu()?.into()
            }
            RdpsndState::WaitingForTrainingConfirm => {
                let pdu::ClientAudioOutputPdu::TrainingConfirm(_) = pdu else {
                    error!("Invalid PDU");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                let client_format = self.client_format.as_ref().expect("available in this state");
                // Formats common to server and client, in the server's
                // preference order, each tagged with its wFormatNo (its
                // position in the *client's* list). Keeping this in the crate
                // means the handler never does index arithmetic and can't emit
                // an out-of-range wFormatNo.
                let common = negotiate_formats(self.handler.get_formats(), &client_format.formats);
                self.state = RdpsndState::Ready;
                self.format_no = if common.is_empty() {
                    debug!("No audio format in common with the client; audio disabled");
                    None
                } else if let Some(chosen) = self.handler.choose_format(&common) {
                    // `chosen` borrows `common` (not `self`), so the encoder
                    // is read off it and the handler is free to borrow again
                    // for the `start` lifecycle hook.
                    let wformat_no = chosen.wformat_no;
                    self.handler.start(chosen);
                    Some(wformat_no)
                } else {
                    debug!("Handler declined every common audio format; audio disabled");
                    None
                };
                vec![]
            }
            RdpsndState::Ready => {
                if let pdu::ClientAudioOutputPdu::WaveConfirm(c) = pdu {
                    debug!(?c);
                }
                vec![]
            }
            state => {
                error!(?state, "Invalid state");
                vec![]
            }
        };
        Ok(msg)
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.state != RdpsndState::Start {
            error!("Attempted to start rdpsnd channel in invalid state");
        }

        let pdu = pdu::ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
            version: pdu::Version::V8,
            formats: self.handler.get_formats().into(),
        });

        self.state = RdpsndState::WaitingForClientFormats;
        Ok(vec![SvcMessage::from(pdu)])
    }
}

impl Drop for RdpsndServer {
    fn drop(&mut self) {
        self.handler.stop();
    }
}

impl SvcServerProcessor for RdpsndServer {}

#[cfg(test)]
mod tests {
    use super::{audio_format_eq, negotiate_formats};
    use crate::pdu::{AudioFormat, WaveFormat};

    fn fmt(format: WaveFormat, rate: u32) -> AudioFormat {
        AudioFormat {
            format,
            n_channels: 2,
            n_samples_per_sec: rate,
            n_avg_bytes_per_sec: rate * 4,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        }
    }

    #[test]
    fn wformat_no_addresses_the_client_list_not_the_server_list() {
        // Server prefers AAC over PCM; the client lists them in the opposite
        // order. wFormatNo must follow the CLIENT's indices.
        let server = [fmt(WaveFormat::AAC_MS, 44100), fmt(WaveFormat::PCM, 44100)];
        let client = [fmt(WaveFormat::PCM, 44100), fmt(WaveFormat::AAC_MS, 44100)];

        let common = negotiate_formats(&server, &client);

        // Ordering follows the server's preference (AAC first)...
        assert_eq!(common.len(), 2);
        assert_eq!(common[0].format().format, WaveFormat::AAC_MS);
        assert_eq!(common[1].format().format, WaveFormat::PCM);
        // ...but each wFormatNo is the position in the CLIENT list.
        assert_eq!(common[0].wformat_no, 1); // AAC is client index 1
        assert_eq!(common[1].wformat_no, 0); // PCM is client index 0
    }

    #[test]
    fn pcm_only_client_gets_a_valid_client_index() {
        // Regression for the --enable-aac trap: server advertises [AAC, PCM]
        // but a PCM-only client must get wFormatNo 0 (its sole index), not
        // PCM's server-list index of 1 (which the client would reject).
        let server = [fmt(WaveFormat::AAC_MS, 44100), fmt(WaveFormat::PCM, 44100)];
        let client = [fmt(WaveFormat::PCM, 44100)];

        let common = negotiate_formats(&server, &client);

        assert_eq!(common.len(), 1);
        assert_eq!(common[0].format().format, WaveFormat::PCM);
        assert_eq!(common[0].wformat_no, 0);
    }

    #[test]
    fn no_shared_format_yields_empty() {
        let server = [fmt(WaveFormat::OPUS, 48000)];
        let client = [fmt(WaveFormat::PCM, 44100)];
        assert!(negotiate_formats(&server, &client).is_empty());
    }

    #[test]
    fn equality_uses_waveformatex_identity_only() {
        let mut a = fmt(WaveFormat::PCM, 44100);
        let mut b = fmt(WaveFormat::PCM, 44100);
        // Differ only in derived/codec fields — still the same format.
        b.n_avg_bytes_per_sec = 0;
        b.n_block_align = 99;
        a.data = Some(vec![1, 2, 3]);
        b.data = None;
        assert!(audio_format_eq(&a, &b));

        // A differing identity field (sample rate) is a different format.
        let c = fmt(WaveFormat::PCM, 48000);
        assert!(!audio_format_eq(&a, &c));
    }
}
