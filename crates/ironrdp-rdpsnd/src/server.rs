use ironrdp_core::{impl_as_any, Decode, ReadCursor};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{decode_err, pdu_other_err, PduResult};
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

pub trait RdpsndServerHandler: Send + core::fmt::Debug {
    fn get_formats(&self) -> &[pdu::AudioFormat];

    fn start(&mut self, client_format: &ClientAudioFormatPdu) -> Option<u16>;

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
            pdu::ServerAudioOutputPdu::Training(pdu).into()
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
            pdu::ServerAudioOutputPdu::Volume(pdu).into()
        ]))
    }

    pub fn close(&mut self) -> PduResult<RdpsndSvcMessages> {
        Ok(RdpsndSvcMessages::new(vec![pdu::ServerAudioOutputPdu::Close.into()]))
    }
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
                self.state = RdpsndState::Ready;
                self.format_no = self.handler.start(client_format);
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
