use std::collections::HashSet;

use ironrdp_connector::{ConnectorError, ConnectorErrorExt, ConnectorResult, Sequence, State, Written};
use ironrdp_pdu as pdu;
use pdu::mcs;

#[derive(Debug)]
pub struct ChannelConnectionSequence {
    state: ChannelConnectionState,
    user_channel_id: u16,
    channels: HashSet<u16>,
}

#[derive(Default, Debug)]
pub enum ChannelConnectionState {
    #[default]
    Consumed,

    WaitErectDomainRequest,
    WaitAttachUserRequest,
    SendAttachUserConfirm,
    WaitChannelJoinRequest {
        joined: HashSet<u16>,
    },
    SendChannelJoinConfirm {
        joined: HashSet<u16>,
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

                debug!(message =? erect_domain_request, "Received");

                (Written::Nothing, ChannelConnectionState::WaitAttachUserRequest)
            }

            ChannelConnectionState::WaitAttachUserRequest => {
                let attach_user_request =
                    ironrdp_pdu::decode::<mcs::AttachUserRequest>(input).map_err(ConnectorError::pdu)?;

                debug!(message =? attach_user_request, "Received");

                (Written::Nothing, ChannelConnectionState::SendAttachUserConfirm)
            }

            ChannelConnectionState::SendAttachUserConfirm => {
                let attach_user_confirm = mcs::AttachUserConfirm {
                    result: 0,
                    initiator_id: self.user_channel_id,
                };

                debug!(message =? attach_user_confirm, "Send");

                let written = ironrdp_pdu::encode_buf(&attach_user_confirm, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    ChannelConnectionState::WaitChannelJoinRequest { joined: HashSet::new() },
                )
            }

            // TODO: support RNS_UD_CS_SUPPORT_SKIP_CHANNELJOIN
            ChannelConnectionState::WaitChannelJoinRequest { joined } => {
                let channel_request =
                    ironrdp_pdu::decode::<mcs::ChannelJoinRequest>(input).map_err(ConnectorError::pdu)?;

                debug!(message =? channel_request, "Received");

                let channel_id = channel_request.channel_id;

                (
                    Written::Nothing,
                    ChannelConnectionState::SendChannelJoinConfirm { joined, channel_id },
                )
            }

            ChannelConnectionState::SendChannelJoinConfirm { mut joined, channel_id } => {
                let channel_confirm = mcs::ChannelJoinConfirm {
                    result: 0,
                    initiator_id: self.user_channel_id,
                    requested_channel_id: channel_id,
                    channel_id,
                };

                debug!(message =? channel_confirm, "Send");

                let written = ironrdp_pdu::encode_buf(&channel_confirm, output).map_err(ConnectorError::pdu)?;

                joined.insert(channel_id);

                let state = if joined != self.channels {
                    ChannelConnectionState::WaitChannelJoinRequest { joined }
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
    pub fn new(user_channel_id: u16, io_channel_id: u16, other_channels: Vec<u16>) -> Self {
        Self {
            state: ChannelConnectionState::WaitErectDomainRequest,
            user_channel_id,
            channels: vec![user_channel_id, io_channel_id]
                .into_iter()
                .chain(other_channels.into_iter())
                .collect(),
        }
    }

    pub fn is_done(&self) -> bool {
        self.state.is_terminal()
    }
}
