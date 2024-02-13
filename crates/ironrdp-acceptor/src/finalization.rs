use std::io::Cursor;

use ironrdp_connector::{ConnectorError, ConnectorErrorExt, ConnectorResult, Sequence, State, Written};
use ironrdp_pdu as pdu;
use pdu::write_buf::WriteBuf;
use pdu::{rdp, PduParsing};

use crate::util::{self, wrap_share_data};

#[derive(Debug)]
pub struct FinalizationSequence {
    state: FinalizationState,
    user_channel_id: u16,
    io_channel_id: u16,

    pub input_events: Vec<Vec<u8>>,
}

#[derive(Default, Debug)]
pub enum FinalizationState {
    #[default]
    Consumed,

    WaitSynchronize,
    WaitControlCooperate,
    WaitRequestControl,
    WaitFontList,

    SendSynchronizeConfirm,
    SendControlCooperateConfirm,
    SendGrantedControlConfirm,
    SendFontMap,

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
            Self::SendSynchronizeConfirm => "SendSynchronizeConfirm",
            Self::SendControlCooperateConfirm => "SendControlCooperateConfirm",
            Self::SendGrantedControlConfirm => "SendGrantedControlConfirm",
            Self::SendFontMap => "SendFontMap",
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
            FinalizationState::WaitFontList => Some(&pdu::RdpHint),
            FinalizationState::SendSynchronizeConfirm => None,
            FinalizationState::SendControlCooperateConfirm => None,
            FinalizationState::SendGrantedControlConfirm => None,
            FinalizationState::SendFontMap => None,
            FinalizationState::Finished => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match std::mem::take(&mut self.state) {
            FinalizationState::WaitSynchronize => {
                let synchronize = decode_share_control(input);

                debug!(message = ?synchronize, "Received");

                (Written::Nothing, FinalizationState::WaitControlCooperate)
            }

            FinalizationState::WaitControlCooperate => {
                let cooperate = decode_share_control(input);

                debug!(message = ?cooperate, "Received");

                (Written::Nothing, FinalizationState::WaitRequestControl)
            }

            FinalizationState::WaitRequestControl => {
                let control = decode_share_control(input)?;

                debug!(message = ?control, "Received");

                (Written::Nothing, FinalizationState::WaitFontList)
            }

            FinalizationState::WaitFontList => match decode_font_list(input) {
                Ok(font_list) => {
                    debug!(message = ?font_list, "Received");

                    (Written::Nothing, FinalizationState::SendSynchronizeConfirm)
                }

                Err(()) => {
                    self.input_events.push(input.to_vec());

                    (Written::Nothing, FinalizationState::WaitFontList)
                }
            },

            FinalizationState::SendSynchronizeConfirm => {
                let synchronize_confirm = create_synchronize_confirm();

                debug!(message = ?synchronize_confirm, "Send");

                let share_data = wrap_share_data(synchronize_confirm, self.io_channel_id);
                let written = util::legacy::encode_send_data_indication(
                    self.user_channel_id,
                    self.io_channel_id,
                    &share_data,
                    output,
                )?;

                (
                    Written::from_size(written)?,
                    FinalizationState::SendControlCooperateConfirm,
                )
            }

            FinalizationState::SendControlCooperateConfirm => {
                let cooperate_confirm = create_cooperate_confirm();

                debug!(message = ?cooperate_confirm, "Send");

                let share_data = wrap_share_data(cooperate_confirm, self.io_channel_id);
                let written = util::legacy::encode_send_data_indication(
                    self.user_channel_id,
                    self.io_channel_id,
                    &share_data,
                    output,
                )?;

                (
                    Written::from_size(written)?,
                    FinalizationState::SendGrantedControlConfirm,
                )
            }

            FinalizationState::SendGrantedControlConfirm => {
                let control_confirm = create_control_confirm(self.user_channel_id);

                debug!(message = ?control_confirm, "Send");

                let share_data = wrap_share_data(control_confirm, self.io_channel_id);
                let written = util::legacy::encode_send_data_indication(
                    self.user_channel_id,
                    self.io_channel_id,
                    &share_data,
                    output,
                )?;

                (Written::from_size(written)?, FinalizationState::SendFontMap)
            }

            FinalizationState::SendFontMap => {
                let font_map = create_font_map();

                debug!(message = ?font_map, "Send");

                let share_data = wrap_share_data(font_map, self.io_channel_id);
                let written = util::legacy::encode_send_data_indication(
                    self.user_channel_id,
                    self.io_channel_id,
                    &share_data,
                    output,
                )?;

                (Written::from_size(written)?, FinalizationState::Finished)
            }

            _ => unreachable!(),
        };

        self.state = next_state;
        Ok(written)
    }
}

impl FinalizationSequence {
    pub fn new(user_channel_id: u16, io_channel_id: u16) -> Self {
        Self {
            state: FinalizationState::WaitSynchronize,
            user_channel_id,
            io_channel_id,
            input_events: Vec::new(),
        }
    }

    pub fn is_done(&self) -> bool {
        self.state.is_terminal()
    }
}

fn create_synchronize_confirm() -> rdp::headers::ShareDataPdu {
    rdp::headers::ShareDataPdu::Synchronize(rdp::finalization_messages::SynchronizePdu { target_user_id: 0 })
}

fn create_cooperate_confirm() -> rdp::headers::ShareDataPdu {
    rdp::headers::ShareDataPdu::Control(rdp::finalization_messages::ControlPdu {
        action: rdp::finalization_messages::ControlAction::Cooperate,
        grant_id: 0,
        control_id: 0,
    })
}

fn create_control_confirm(user_id: u16) -> rdp::headers::ShareDataPdu {
    rdp::headers::ShareDataPdu::Control(rdp::finalization_messages::ControlPdu {
        action: rdp::finalization_messages::ControlAction::GrantedControl,
        grant_id: user_id,
        control_id: u32::from(pdu::rdp::capability_sets::SERVER_CHANNEL_ID),
    })
}

fn create_font_map() -> rdp::headers::ShareDataPdu {
    rdp::headers::ShareDataPdu::FontMap(rdp::finalization_messages::FontPdu::default())
}

fn decode_share_control(input: &[u8]) -> ConnectorResult<rdp::headers::ShareControlHeader> {
    let data_request = pdu::decode::<pdu::mcs::SendDataRequest<'_>>(input).map_err(ConnectorError::pdu)?;
    let share_control = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data_request.user_data))?;
    Ok(share_control)
}

fn decode_font_list(input: &[u8]) -> Result<rdp::finalization_messages::FontPdu, ()> {
    use pdu::rdp::headers::{ShareControlPdu, ShareDataPdu};

    let share_control = decode_share_control(input).map_err(|_| ())?;

    let ShareControlPdu::Data(data_pdu) = share_control.share_control_pdu else {
        return Err(());
    };

    let ShareDataPdu::FontList(font_pdu) = data_pdu.share_data_pdu else {
        return Err(());
    };

    Ok(font_pdu)
}
