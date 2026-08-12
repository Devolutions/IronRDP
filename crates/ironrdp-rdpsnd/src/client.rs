use std::borrow::Cow;

use ironrdp_core::{Decode as _, EncodeResult, ReadCursor, cast_length, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use tracing::{debug, error, warn};

use crate::pdu::{self, AudioFormat, PitchPdu, ServerAudioFormatPdu, TrainingPdu, VolumePdu};
use crate::server::RdpsndSvcMessages;

pub trait RdpsndClientHandler: Send + core::fmt::Debug {
    fn get_flags(&self) -> pdu::AudioFormatFlags {
        pdu::AudioFormatFlags::empty()
    }

    fn get_formats(&self) -> &[AudioFormat];

    /// Play a wave block for the given negotiated format.
    ///
    /// `format` is the entry from the client format list advertised during
    /// negotiation (MS-RDPEA `wFormatNo` indexes that list).
    fn wave(&mut self, format: &AudioFormat, ts: u32, data: Cow<'_, [u8]>);

    fn set_volume(&mut self, volume: VolumePdu);

    fn set_pitch(&mut self, pitch: PitchPdu);

    fn close(&mut self);
}

#[derive(Debug)]
pub struct NoopRdpsndBackend;

impl RdpsndClientHandler for NoopRdpsndBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &[]
    }

    fn wave(&mut self, _format: &AudioFormat, _ts: u32, _data: Cow<'_, [u8]>) {}

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RdpsndState {
    Start,
    WaitingForTraining,
    Ready,
    Stop,
}

/// Required for rdpdr to work: [\[MS-RDPEFS\] Appendix A<1>]
///
/// [\[MS-RDPEFS\] Appendix A<1>]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1
#[derive(Debug)]
pub struct Rdpsnd {
    handler: Box<dyn RdpsndClientHandler>,
    state: RdpsndState,
    server_format: Option<ServerAudioFormatPdu>,
    /// Formats advertised to the server, in wire order.
    ///
    /// Wave/Wave2 `wFormatNo` indexes this list (MS-RDPEA).
    client_formats: Vec<AudioFormat>,
}

impl Rdpsnd {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

    pub fn new(handler: Box<dyn RdpsndClientHandler>) -> Self {
        Self {
            handler,
            state: RdpsndState::Start,
            server_format: None,
            client_formats: Vec::new(),
        }
    }

    pub fn get_format(&self, format_no: u16) -> PduResult<&AudioFormat> {
        self.client_formats
            .get(usize::from(format_no))
            .ok_or_else(|| pdu_other_err!("invalid format"))
    }

    pub fn version(&self) -> PduResult<pdu::Version> {
        let server_format = self
            .server_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state - no version"))?;

        Ok(server_format.version)
    }

    pub fn client_formats(&mut self) -> PduResult<RdpsndSvcMessages> {
        // Windows seems to be confused if the client replies with more formats, or unknown formats (e.g.: opus).
        // Keep only formats also offered by the server, preserving handler order so
        // wFormatNo stays a stable index into this reply list.
        let server_formats = &self
            .server_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state - no server format"))?
            .formats;

        let formats: Vec<AudioFormat> = self
            .handler
            .get_formats()
            .iter()
            .filter(|client_fmt| {
                server_formats
                    .iter()
                    .any(|server_fmt| client_fmt.matches_for_negotiation(server_fmt))
            })
            .cloned()
            .collect();

        self.client_formats = formats.clone();

        let pdu = pdu::ClientAudioFormatPdu {
            version: self.version()?,
            flags: self.handler.get_flags() | pdu::AudioFormatFlags::ALIVE,
            formats,
            volume_left: 0xFFFF,
            volume_right: 0xFFFF,
            pitch: 0x00010000,
            dgram_port: 0,
        };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::AudioFormat(pdu).into(),
        ]))
    }

    pub fn quality_mode(&mut self) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::QualityModePdu {
            quality_mode: pdu::QualityMode::High,
        };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::QualityMode(pdu).into(),
        ]))
    }

    pub fn training_confirm(&mut self, pdu: &TrainingPdu) -> PduResult<RdpsndSvcMessages> {
        let pack_size: EncodeResult<_> = cast_length!("wPackSize", pdu.data.len());
        let pack_size = pack_size.map_err(|e| encode_err!(e))?;
        let pdu = pdu::TrainingConfirmPdu {
            timestamp: pdu.timestamp,
            pack_size,
        };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::TrainingConfirm(pdu).into(),
        ]))
    }

    pub fn wave_confirm(&mut self, timestamp: u16, block_no: u8) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::WaveConfirmPdu { timestamp, block_no };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::WaveConfirm(pdu).into(),
        ]))
    }

    fn play_wave(&mut self, format_no: u16, ts: u32, data: Cow<'_, [u8]>) {
        match self.client_formats.get(usize::from(format_no)) {
            Some(format) => {
                // Clone so the handler can hold the format across mutable self use.
                let format = format.clone();
                self.handler.wave(&format, ts, data);
            }
            None => {
                warn!(
                    format_no,
                    n_formats = self.client_formats.len(),
                    "Ignoring wave with out-of-range format_no"
                );
            }
        }
    }

    fn begin_format_negotiation(&mut self, af: ServerAudioFormatPdu) -> PduResult<Vec<SvcMessage>> {
        self.handler.close();
        self.server_format = Some(af);
        self.client_formats.clear();
        self.state = RdpsndState::WaitingForTraining;
        let mut msgs: Vec<SvcMessage> = self.client_formats()?.into();
        if self.version()? >= pdu::Version::V6 {
            let mut m = self.quality_mode()?.into();
            msgs.append(&mut m);
        }
        Ok(msgs)
    }
}

impl_as_any!(Rdpsnd);

impl SvcProcessor for Rdpsnd {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = match pdu::ServerAudioOutputPdu::decode(&mut ReadCursor::new(payload)) {
            Ok(pdu) => pdu,
            Err(error) => {
                error!(?error, "Ignoring malformed RDPSND PDU");
                return Ok(vec![]);
            }
        };

        debug!(?pdu, ?self.state);
        let msg = match self.state {
            RdpsndState::Start => {
                let pdu::ServerAudioOutputPdu::AudioFormat(af) = pdu else {
                    error!("Invalid pdu");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                self.begin_format_negotiation(af)?
            }
            RdpsndState::WaitingForTraining => {
                let pdu::ServerAudioOutputPdu::Training(pdu) = pdu else {
                    error!("Invalid PDU");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                self.state = RdpsndState::Ready;
                self.training_confirm(&pdu)?.into()
            }
            RdpsndState::Ready => {
                match pdu {
                    pdu::ServerAudioOutputPdu::Wave(pdu) => {
                        // Pre-v8 Wave: concatenated WaveInfo + Wave payload.
                        // audio_timestamp is not present; use protocol timestamp.
                        let ts = u32::from(pdu.timestamp);
                        self.play_wave(pdu.format_no, ts, pdu.data);
                        return Ok(self.wave_confirm(pdu.timestamp, pdu.block_no)?.into());
                    }
                    pdu::ServerAudioOutputPdu::Wave2(pdu) => {
                        self.play_wave(pdu.format_no, pdu.audio_timestamp, pdu.data);
                        return Ok(self.wave_confirm(pdu.timestamp, pdu.block_no)?.into());
                    }
                    pdu::ServerAudioOutputPdu::Volume(pdu) => {
                        self.handler.set_volume(pdu);
                    }
                    pdu::ServerAudioOutputPdu::Pitch(pdu) => {
                        self.handler.set_pitch(pdu);
                    }
                    pdu::ServerAudioOutputPdu::Close => {
                        self.handler.close();
                    }
                    pdu::ServerAudioOutputPdu::Training(pdu) => return Ok(self.training_confirm(&pdu)?.into()),
                    pdu::ServerAudioOutputPdu::AudioFormat(af) => {
                        return self.begin_format_negotiation(af);
                    }
                    // Unsupported optional PDUs: keep the channel alive.
                    pdu::ServerAudioOutputPdu::CryptKey(_) | pdu::ServerAudioOutputPdu::WaveEncrypt(_) => {
                        warn!(?pdu, "Ignoring unsupported RDPSND PDU");
                    }
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
}

impl Drop for Rdpsnd {
    fn drop(&mut self) {
        self.handler.close();
    }
}

impl SvcClientProcessor for Rdpsnd {}
