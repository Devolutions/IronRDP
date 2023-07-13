use ironrdp_connector::{ConnectorError, ConnectorErrorExt, ConnectorResult, Sequence, State, Written};
use ironrdp_pdu as pdu;
use pdu::mcs;

#[derive(Debug)]
pub struct ChannelConnectionSequence {
    state: ChannelConnectionState,
    io_channel_id: u16,
    user_channel_id: u16,
    channels: Vec<u16>,
}

#[derive(Default, Debug)]
pub enum ChannelConnectionState {
    #[default]
    Consumed,

    WaitErectDomainRequest,
    WaitAttachUserRequest,
    SendAttachUserConfirm,
    WaitChannelJoinRequest {
        index: usize,
    },
    SendChannelJoinConfirm {
        index: usize,
        channel_id: u16,
    },
    AllJoined,
}

impl State for ChannelConnectionState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::WaitErectDomainRequest => "WaitErectDomainRequest",
            Self::WaitAttachUserRequest => "WaitAttachUserRequest",
            Self::SendAttachUserConfirm => "SendAttachUserConfirm",
            Self::WaitChannelJoinRequest { .. } => "WaitChannelJoinRequest",
            Self::SendChannelJoinConfirm { .. } => "SendChannelJoinConfirm",
            Self::AllJoined { .. } => "AllJoined",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::AllJoined { .. })
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl Sequence for ChannelConnectionSequence {
    fn next_pdu_hint(&self) -> Option<&dyn pdu::PduHint> {
        match &self.state {
            ChannelConnectionState::Consumed => None,
            ChannelConnectionState::WaitErectDomainRequest => Some(&pdu::X224_HINT),
            ChannelConnectionState::WaitAttachUserRequest => Some(&pdu::X224_HINT),
            ChannelConnectionState::SendAttachUserConfirm => None,
            ChannelConnectionState::WaitChannelJoinRequest { .. } => Some(&pdu::X224_HINT),
            ChannelConnectionState::SendChannelJoinConfirm { .. } => None,
            ChannelConnectionState::AllJoined { .. } => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut Vec<u8>) -> ConnectorResult<Written> {
        let (written, next_state) = match std::mem::take(&mut self.state) {
            ChannelConnectionState::WaitErectDomainRequest => {
                let erect_domain_request =
                    ironrdp_pdu::decode::<mcs::ErectDomainPdu>(input).map_err(ConnectorError::pdu)?;

                println!("{:?}", erect_domain_request);

                (Written::Nothing, ChannelConnectionState::WaitAttachUserRequest)
            }

            ChannelConnectionState::WaitAttachUserRequest => {
                let attach_user_request =
                    ironrdp_pdu::decode::<mcs::AttachUserRequest>(input).map_err(ConnectorError::pdu)?;

                println!("{:?}", attach_user_request);

                (Written::Nothing, ChannelConnectionState::SendAttachUserConfirm)
            }

            ChannelConnectionState::SendAttachUserConfirm => {
                let attach_user_confirm = mcs::AttachUserConfirm {
                    result: 0,
                    initiator_id: self.user_channel_id,
                };

                println!("{:?}", attach_user_confirm);

                let written = ironrdp_pdu::encode_buf(&attach_user_confirm, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    ChannelConnectionState::WaitChannelJoinRequest { index: 0 },
                )
            }

            ChannelConnectionState::WaitChannelJoinRequest { index } => {
                let channel_request =
                    ironrdp_pdu::decode::<mcs::ChannelJoinRequest>(input).map_err(ConnectorError::pdu)?;

                println!("{:?}", channel_request);

                let channel_id = channel_request.channel_id;

                (
                    Written::Nothing,
                    ChannelConnectionState::SendChannelJoinConfirm { index, channel_id },
                )
            }

            ChannelConnectionState::SendChannelJoinConfirm { index, channel_id } => {
                let channel_confirm = mcs::ChannelJoinConfirm {
                    result: 0,
                    initiator_id: self.user_channel_id,
                    requested_channel_id: channel_id,
                    channel_id,
                };

                println!("{:?}", channel_confirm);

                let written = ironrdp_pdu::encode_buf(&channel_confirm, output).map_err(ConnectorError::pdu)?;

                let state = if self.channels.len() > index + 1 {
                    ChannelConnectionState::WaitChannelJoinRequest { index: index + 1 }
                } else {
                    ChannelConnectionState::AllJoined {}
                };

                (Written::from_size(written)?, state)
            }

            _ => unreachable!(),
        };

        self.state = next_state;
        Ok(written)
    }
}

impl ChannelConnectionSequence {
    pub fn new(io_channel_id: u16, user_channel_id: u16, channels: Vec<u16>) -> Self {
        Self {
            state: ChannelConnectionState::WaitErectDomainRequest,
            io_channel_id,
            user_channel_id,
            channels,
        }
    }

    pub fn is_done(&self) -> bool {
        self.state.is_terminal()
    }
}
