use std::borrow::Cow;

use ironrdp_core::impl_as_any;
use ironrdp_core::ReadCursor;
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{cast_length, other_err, PduDecode, PduResult};
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use tracing::{debug, error};

use crate::pdu::{self, AudioFormat, PitchPdu, ServerAudioFormatPdu, TrainingPdu, VolumePdu};
use crate::server::RdpsndSvcMessages;

pub trait RdpsndClientHandler: Send + std::fmt::Debug {
    fn wave(&mut self, format: &AudioFormat, ts: u32, data: Cow<'_, [u8]>);

    fn set_volume(&mut self, volume: VolumePdu);

    fn set_pitch(&mut self, pitch: PitchPdu);

    fn close(&mut self);
}

#[derive(Debug)]
pub struct NoopRdpsndBackend;

impl RdpsndClientHandler for NoopRdpsndBackend {
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
}

impl Rdpsnd {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

    pub fn new(handler: Box<dyn RdpsndClientHandler>) -> Self {
        Self {
            handler,
            state: RdpsndState::Start,
            server_format: None,
        }
    }

    pub fn get_format(&self, format_no: u16) -> PduResult<&AudioFormat> {
        let server_format = self
            .server_format
            .as_ref()
            .ok_or(other_err!("invalid state - no format"))?;

        server_format
            .formats
            .get(format_no as usize)
            .ok_or(other_err!("invalid format"))
    }

    pub fn version(&self) -> PduResult<pdu::Version> {
        let server_format = self
            .server_format
            .as_ref()
            .ok_or(other_err!("invalid state - no version"))?;

        Ok(server_format.version)
    }

    pub fn client_formats(&mut self) -> PduResult<RdpsndSvcMessages> {
        let server_format = self
            .server_format
            .as_ref()
            .ok_or(other_err!("invalid state - no format"))?;

        let pdu = pdu::ClientAudioFormatPdu {
            version: self.version()?,
            flags: pdu::AudioFormatFlags::empty(),
            formats: server_format.formats.clone(),
            volume_left: 0xFFFF,
            volume_right: 0xFFFF,
            pitch: 0x00010000,
            dgram_port: 0,
        };
        Ok(RdpsndSvcMessages::new(vec![pdu::ClientAudioOutputPdu::AudioFormat(
            pdu,
        )
        .into()]))
    }

    pub fn quality_mode(&mut self) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::QualityModePdu {
            quality_mode: pdu::QualityMode::High,
        };
        Ok(RdpsndSvcMessages::new(vec![pdu::ClientAudioOutputPdu::QualityMode(
            pdu,
        )
        .into()]))
    }

    pub fn training_confirm(&mut self, pdu: &TrainingPdu) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::TrainingConfirmPdu {
            timestamp: pdu.timestamp,
            pack_size: cast_length!("wPackSize", pdu.data.len())?,
        };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::TrainingConfirm(pdu).into(),
        ]))
    }

    pub fn wave_confirm(&mut self, timestamp: u16, block_no: u8) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::WaveConfirmPdu { timestamp, block_no };
        Ok(RdpsndSvcMessages::new(vec![pdu::ClientAudioOutputPdu::WaveConfirm(
            pdu,
        )
        .into()]))
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
        let pdu = pdu::ServerAudioOutputPdu::decode(&mut ReadCursor::new(payload))?;

        debug!(?pdu, ?self.state);
        let msg = match self.state {
            RdpsndState::Start => {
                let pdu::ServerAudioOutputPdu::AudioFormat(af) = pdu else {
                    error!("Invalid pdu");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                self.server_format = Some(af);
                self.state = RdpsndState::WaitingForTraining;
                let mut msgs: Vec<SvcMessage> = self.client_formats()?.into();
                if self.version()? >= pdu::Version::V6 {
                    let mut m = self.quality_mode()?.into();
                    msgs.append(&mut m);
                }
                msgs
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
                    // TODO: handle WaveInfo for < v8
                    pdu::ServerAudioOutputPdu::Wave2(pdu) => {
                        let fmt = self.get_format(pdu.format_no)?.clone();
                        let ts = pdu.audio_timestamp;
                        self.handler.wave(&fmt, ts, pdu.data);
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
                    _ => {
                        error!("Invalid PDU");
                        self.state = RdpsndState::Stop;
                        return Ok(vec![]);
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
