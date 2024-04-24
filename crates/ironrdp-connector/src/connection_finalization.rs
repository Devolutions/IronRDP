use std::mem;

use ironrdp_pdu::rdp::capability_sets::SERVER_CHANNEL_ID;
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::rdp::{finalization_messages, server_error_info};
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::PduHint;

use crate::{legacy, ConnectorResult, Sequence, State, Written};

#[derive(Default, Debug, Clone)]
#[non_exhaustive]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum ConnectionFinalizationState {
    #[default]
    Consumed,

    SendSynchronize,
    SendControlCooperate,
    SendRequestControl,
    SendFontList,

    WaitForResponse,

    Finished,
}

impl State for ConnectionFinalizationState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::SendSynchronize => "SendSynchronize",
            Self::SendControlCooperate => "SendControlCooperate",
            Self::SendRequestControl => "SendRequestControl",
            Self::SendFontList => "SendFontList",
            Self::WaitForResponse => "WaitForResponse",
            Self::Finished => "Finished",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished)
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ConnectionFinalizationSequence {
    pub state: ConnectionFinalizationState,
    pub io_channel_id: u16,
    pub user_channel_id: u16,
}

impl ConnectionFinalizationSequence {
    pub fn new(io_channel_id: u16, user_channel_id: u16) -> Self {
        Self {
            state: ConnectionFinalizationState::SendSynchronize,
            io_channel_id,
            user_channel_id,
        }
    }
}

impl Sequence for ConnectionFinalizationSequence {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match self.state {
            ConnectionFinalizationState::Consumed => None,
            ConnectionFinalizationState::SendSynchronize => None,
            ConnectionFinalizationState::SendControlCooperate => None,
            ConnectionFinalizationState::SendRequestControl => None,
            ConnectionFinalizationState::SendFontList => None,
            ConnectionFinalizationState::WaitForResponse => Some(&ironrdp_pdu::X224_HINT),
            ConnectionFinalizationState::Finished => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            ConnectionFinalizationState::Consumed => {
                return Err(general_err!(
                    "connection finalization sequence state is consumed (this is a bug)",
                ))
            }

            ConnectionFinalizationState::SendSynchronize => {
                let message = ShareDataPdu::Synchronize(finalization_messages::SynchronizePdu {
                    target_user_id: self.user_channel_id,
                });

                debug!(?message, "Send");

                let written = legacy::encode_share_data(self.user_channel_id, self.io_channel_id, 0, message, output)?;

                (
                    Written::from_size(written)?,
                    ConnectionFinalizationState::SendControlCooperate,
                )
            }

            ConnectionFinalizationState::SendControlCooperate => {
                let message = ShareDataPdu::Control(finalization_messages::ControlPdu {
                    action: finalization_messages::ControlAction::Cooperate,
                    grant_id: 0,
                    control_id: 0,
                });

                debug!(?message, "Send");

                let written = legacy::encode_share_data(self.user_channel_id, self.io_channel_id, 0, message, output)?;

                (
                    Written::from_size(written)?,
                    ConnectionFinalizationState::SendRequestControl,
                )
            }

            ConnectionFinalizationState::SendRequestControl => {
                let message = ShareDataPdu::Control(finalization_messages::ControlPdu {
                    action: finalization_messages::ControlAction::RequestControl,
                    grant_id: 0,
                    control_id: 0,
                });

                debug!(?message, "Send");

                let written = legacy::encode_share_data(self.user_channel_id, self.io_channel_id, 0, message, output)?;

                (Written::from_size(written)?, ConnectionFinalizationState::SendFontList)
            }

            ConnectionFinalizationState::SendFontList => {
                let message = ShareDataPdu::FontList(finalization_messages::FontPdu::default());

                debug!(?message, "Send");

                let written = legacy::encode_share_data(self.user_channel_id, self.io_channel_id, 0, message, output)?;

                (
                    Written::from_size(written)?,
                    ConnectionFinalizationState::WaitForResponse,
                )
            }

            ConnectionFinalizationState::WaitForResponse => {
                let ctx = legacy::decode_send_data_indication(input)?;
                let ctx = legacy::decode_share_data(ctx)?;

                debug!(message = ?ctx.pdu, "Received");

                let next_state = match ctx.pdu {
                    ShareDataPdu::Synchronize(_) => {
                        debug!("Server Synchronize");
                        ConnectionFinalizationState::WaitForResponse
                    }
                    ShareDataPdu::Control(control_pdu) => {
                        match control_pdu.action {
                            finalization_messages::ControlAction::Cooperate => {
                                if control_pdu.grant_id == 0 && control_pdu.control_id == 0 {
                                    debug!("Server Control (Cooperate)");
                                } else {
                                    warn!(
                                        control_pdu.grant_id,
                                        control_pdu.control_id,
                                        user_channel_id = self.user_channel_id,
                                        "Server Control (Cooperate) has non-zero grant_id or control_id",
                                    );
                                }
                                ConnectionFinalizationState::WaitForResponse
                            }
                            finalization_messages::ControlAction::GrantedControl => {
                                debug!(
                                    control_pdu.grant_id,
                                    control_pdu.control_id,
                                    user_channel_id = self.user_channel_id,
                                    SERVER_CHANNEL_ID
                                );

                                if control_pdu.grant_id != self.user_channel_id {
                                    warn!("Server Control (Granted Control) had invalid grant_id, expected {}, but got {}", self.user_channel_id, control_pdu.grant_id);
                                }

                                if control_pdu.control_id != u32::from(SERVER_CHANNEL_ID) {
                                    warn!("Server Control (Granted Control) had invalid control_id, expected {}, but got {}", SERVER_CHANNEL_ID, control_pdu.control_id);
                                }

                                ConnectionFinalizationState::WaitForResponse
                            }
                            _ => return Err(general_err!("unexpected control action")),
                        }
                    }
                    ShareDataPdu::ServerSetErrorInfo(server_error_info::ServerSetErrorInfoPdu(error_info)) => {
                        match error_info {
                            server_error_info::ErrorInfo::ProtocolIndependentCode(
                                server_error_info::ProtocolIndependentCode::None,
                            ) => ConnectionFinalizationState::WaitForResponse,
                            _ => {
                                return Err(reason_err!(
                                    "ServerSetErrorInfo",
                                    "server returned error info: {}",
                                    error_info.description()
                                ));
                            }
                        }
                    }
                    ShareDataPdu::FontMap(_) => {
                        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/023f1e69-cfe8-4ee6-9ee0-7e759fb4e4ee
                        //
                        // Once the client has sent the Confirm Active PDU, it can start
                        // sending mouse and keyboard input to the server, and upon receipt
                        // of the Font List PDU the server can start sending graphics
                        // output to the client.

                        ConnectionFinalizationState::Finished
                    }
                    _ => return Err(general_err!("unexpected server message")),
                };

                (Written::Nothing, next_state)
            }

            ConnectionFinalizationState::Finished => return Err(general_err!("finalization already finished")),
        };

        self.state = next_state;

        Ok(written)
    }
}
