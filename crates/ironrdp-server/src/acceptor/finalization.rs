use std::{borrow::Cow, io::Cursor};

use ironrdp_connector::{ConnectorError, ConnectorErrorExt, ConnectorResult, Sequence, State, Written};
use ironrdp_pdu as pdu;
use pdu::{rdp, PduParsing};

#[derive(Debug)]
pub struct FinalizationSequence {
    state: FinalizationState,
    user_id: u16,
}

#[derive(Default, Debug)]
pub enum FinalizationState {
    #[default]
    Consumed,

    WaitSynchronize,
    WaitControlCooperate,
    WaitRequestControl,
    WaitFontList,

    SendResponse,

    Finished,
}

impl State for FinalizationState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::WaitSynchronize => "WaitSynchronize",
            Self::WaitControlCooperate => "WaitControlCooperate",
            Self::WaitRequestControl => "WaitRequestControl",
            Self::WaitFontList => "WaitFontList",
            Self::SendResponse => "SendResponse",
            Self::Finished => "Finished",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished { .. })
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl Sequence for FinalizationSequence {
    fn next_pdu_hint(&self) -> Option<&dyn pdu::PduHint> {
        match &self.state {
            FinalizationState::Consumed => None,
            FinalizationState::WaitSynchronize => Some(&pdu::X224Hint),
            FinalizationState::WaitControlCooperate => Some(&pdu::X224Hint),
            FinalizationState::WaitRequestControl => Some(&pdu::X224Hint),
            FinalizationState::WaitFontList => Some(&pdu::X224Hint),
            FinalizationState::SendResponse => None,
            FinalizationState::Finished => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut Vec<u8>) -> ConnectorResult<Written> {
        let (written, next_state) = match std::mem::take(&mut self.state) {
            FinalizationState::WaitSynchronize => {
                let data = pdu::decode::<pdu::mcs::SendDataRequest>(input).map_err(ConnectorError::pdu)?;

                let synchronize = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data.user_data))?;
                println!("{:?}", synchronize);

                (Written::Nothing, FinalizationState::WaitControlCooperate)
            }

            FinalizationState::WaitControlCooperate => {
                let data = pdu::decode::<pdu::mcs::SendDataRequest>(input).map_err(ConnectorError::pdu)?;

                let cooperate = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data.user_data))?;
                println!("{:?}", cooperate);

                (Written::Nothing, FinalizationState::WaitRequestControl)
            }

            FinalizationState::WaitRequestControl => {
                let data = pdu::decode::<pdu::mcs::SendDataRequest>(input).map_err(ConnectorError::pdu)?;

                let control = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data.user_data))?;
                println!("{:?}", control);

                (Written::Nothing, FinalizationState::WaitFontList)
            }

            FinalizationState::WaitFontList => {
                let data = pdu::decode::<pdu::mcs::SendDataRequest>(input).map_err(ConnectorError::pdu)?;

                let font_list = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data.user_data))?;
                println!("{:?}", font_list);

                (Written::Nothing, FinalizationState::SendResponse)
            }

            FinalizationState::SendResponse => {
                let responses = vec![
                    synchronize_confirm(self.user_id),
                    cooperate_confirm(self.user_id),
                    control_confirm(self.user_id),
                    fontmap_confirm(),
                ];

                let mut written = 0;
                for share_pdu in responses {
                    println!("{:?}", share_pdu);

                    let mut buf = Vec::with_capacity(share_pdu.buffer_length());
                    share_pdu.to_buffer(Cursor::new(&mut buf))?;

                    let indication = pdu::mcs::SendDataIndication {
                        initiator_id: self.user_id,
                        channel_id: 0,
                        user_data: Cow::Borrowed(&buf[..share_pdu.buffer_length()]),
                    };

                    written += ironrdp_pdu::encode_buf(&indication, output).map_err(ConnectorError::pdu)?;
                }

                (Written::from_size(written)?, FinalizationState::Finished)
            }

            _ => unreachable!(),
        };

        self.state = next_state;
        Ok(written)
    }
}

impl FinalizationSequence {
    pub fn new(user_id: u16) -> Self {
        Self {
            state: FinalizationState::WaitSynchronize,
            user_id,
        }
    }

    pub fn is_done(&self) -> bool {
        self.state.is_terminal()
    }
}

fn synchronize_confirm(user_id: u16) -> rdp::headers::ShareControlHeader {
    with_share_header(rdp::headers::ShareControlPdu::Data(rdp::headers::ShareDataHeader {
        share_data_pdu: rdp::headers::ShareDataPdu::Synchronize(rdp::finalization_messages::SynchronizePdu {
            target_user_id: user_id,
        }),
        stream_priority: rdp::headers::StreamPriority::Undefined,
        compression_flags: rdp::headers::CompressionFlags::empty(),
        compression_type: rdp::client_info::CompressionType::K8,
    }))
}

fn cooperate_confirm(user_id: u16) -> rdp::headers::ShareControlHeader {
    with_share_header(rdp::headers::ShareControlPdu::Data(rdp::headers::ShareDataHeader {
        share_data_pdu: rdp::headers::ShareDataPdu::Control(rdp::finalization_messages::ControlPdu {
            action: rdp::finalization_messages::ControlAction::Cooperate,
            grant_id: user_id,
            control_id: u32::from(pdu::rdp::capability_sets::SERVER_CHANNEL_ID),
        }),
        stream_priority: rdp::headers::StreamPriority::Undefined,
        compression_flags: rdp::headers::CompressionFlags::empty(),
        compression_type: rdp::client_info::CompressionType::K8,
    }))
}

fn control_confirm(user_id: u16) -> rdp::headers::ShareControlHeader {
    with_share_header(rdp::headers::ShareControlPdu::Data(rdp::headers::ShareDataHeader {
        share_data_pdu: rdp::headers::ShareDataPdu::Control(rdp::finalization_messages::ControlPdu {
            action: rdp::finalization_messages::ControlAction::GrantedControl,
            grant_id: user_id,
            control_id: u32::from(pdu::rdp::capability_sets::SERVER_CHANNEL_ID),
        }),
        stream_priority: rdp::headers::StreamPriority::Undefined,
        compression_flags: rdp::headers::CompressionFlags::empty(),
        compression_type: rdp::client_info::CompressionType::K8,
    }))
}

fn fontmap_confirm() -> rdp::headers::ShareControlHeader {
    with_share_header(rdp::headers::ShareControlPdu::Data(rdp::headers::ShareDataHeader {
        share_data_pdu: rdp::headers::ShareDataPdu::FontMap(rdp::finalization_messages::FontPdu {
            number: 1, // TODO: fields
            total_number: 1,
            flags: rdp::finalization_messages::SequenceFlags::empty(),
            entry_size: 0,
        }),
        stream_priority: rdp::headers::StreamPriority::Undefined,
        compression_flags: rdp::headers::CompressionFlags::empty(),
        compression_type: rdp::client_info::CompressionType::K8,
    }))
}

fn with_share_header(pdu: rdp::headers::ShareControlPdu) -> rdp::headers::ShareControlHeader {
    rdp::headers::ShareControlHeader {
        share_id: 1,
        pdu_source: 1,
        share_control_pdu: pdu,
    }
}
