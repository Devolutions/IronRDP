use std::mem;

use ironrdp_pdu::{mcs, PduHint};

use crate::{ConnectorError, ConnectorErrorExt as _, ConnectorResult, Sequence, State, Written};

#[derive(Default, Debug)]
#[non_exhaustive]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum ChannelConnectionState {
    #[default]
    Consumed,

    SendErectDomainRequest,
    SendAttachUserRequest,
    WaitAttachUserConfirm,
    SendChannelJoinRequest {
        user_channel_id: u16,
        index: usize,
    },
    WaitChannelJoinConfirm {
        user_channel_id: u16,
        index: usize,
    },
    AllJoined {
        user_channel_id: u16,
    },
}

impl State for ChannelConnectionState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::SendErectDomainRequest => "SendErectDomainRequest",
            Self::SendAttachUserRequest => "SendAttachUserRequest",
            Self::WaitAttachUserConfirm => "WaitAttachUserConfirm",
            Self::SendChannelJoinRequest { .. } => "SendChannelJoinRequest",
            Self::WaitChannelJoinConfirm { .. } => "WaitChannelJoinConfirm",
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

#[derive(Debug)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ChannelConnectionSequence {
    pub state: ChannelConnectionState,
    pub channel_ids: Vec<u16>,
}

impl ChannelConnectionSequence {
    pub fn new(io_channel_id: u16, mut channel_ids: Vec<u16>) -> Self {
        // I/O channel ID must be joined as well
        channel_ids.push(io_channel_id);

        Self {
            state: ChannelConnectionState::SendErectDomainRequest,
            channel_ids,
        }
    }
}

impl Sequence for ChannelConnectionSequence {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match self.state {
            ChannelConnectionState::Consumed => None,
            ChannelConnectionState::SendErectDomainRequest => None,
            ChannelConnectionState::SendAttachUserRequest => None,
            ChannelConnectionState::WaitAttachUserConfirm => Some(&ironrdp_pdu::X224_HINT),
            ChannelConnectionState::SendChannelJoinRequest { .. } => None,
            ChannelConnectionState::WaitChannelJoinConfirm { .. } => Some(&ironrdp_pdu::X224_HINT),
            ChannelConnectionState::AllJoined { .. } => None,
        }
    }

    fn step(&mut self, input: &[u8], output: &mut Vec<u8>) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            ChannelConnectionState::Consumed => {
                return Err(general_err!(
                    "channel connection sequence state is consumed (this is a bug)",
                ))
            }

            ChannelConnectionState::SendErectDomainRequest => {
                let erect_domain_request = mcs::ErectDomainPdu {
                    sub_height: 0,
                    sub_interval: 0,
                };

                debug!(message = ?erect_domain_request, "Send");

                let written = ironrdp_pdu::encode_buf(&erect_domain_request, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    ChannelConnectionState::SendAttachUserRequest,
                )
            }

            ChannelConnectionState::SendAttachUserRequest => {
                let attach_user_request = mcs::AttachUserRequest;

                debug!(message = ?attach_user_request, "Send");

                let written = ironrdp_pdu::encode_buf(&attach_user_request, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    ChannelConnectionState::WaitAttachUserConfirm,
                )
            }

            ChannelConnectionState::WaitAttachUserConfirm => {
                let attach_user_confirm =
                    ironrdp_pdu::decode::<mcs::AttachUserConfirm>(input).map_err(ConnectorError::pdu)?;

                let user_channel_id = attach_user_confirm.initiator_id;

                debug!(message = ?attach_user_confirm, user_channel_id, "Received");

                debug_assert!(!self.channel_ids.is_empty());

                (
                    Written::Nothing,
                    ChannelConnectionState::SendChannelJoinRequest {
                        user_channel_id,
                        index: 0,
                    },
                )
            }

            // TODO: send all the join requests in a single batch
            // (RDP 4.0, 5.0, 5.1, 5.2, 6.0, 6.1, 7.0, 7.1, 8.0, 10.2, 10.3,
            // 10.4, and 10.5 clients send a Channel Join Request to the server only after the
            // Channel Join Confirm for a previously sent request has been received. RDP 8.1,
            // 10.0, and 10.1 clients send all of the Channel Join Requests to the server in a
            // single batch to minimize the overall connection sequence time.)
            ChannelConnectionState::SendChannelJoinRequest { user_channel_id, index } => {
                let channel_id = self.channel_ids[index];

                let channel_join_request = mcs::ChannelJoinRequest {
                    initiator_id: user_channel_id,
                    channel_id,
                };

                debug!(message = ?channel_join_request, "Send");

                let written = ironrdp_pdu::encode_buf(&channel_join_request, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    ChannelConnectionState::WaitChannelJoinConfirm { user_channel_id, index },
                )
            }

            ChannelConnectionState::WaitChannelJoinConfirm { user_channel_id, index } => {
                let channel_id = self.channel_ids[index];

                let channel_join_confirm =
                    ironrdp_pdu::decode::<mcs::ChannelJoinConfirm>(input).map_err(ConnectorError::pdu)?;

                debug!(message = ?channel_join_confirm, "Received");

                if channel_join_confirm.initiator_id != user_channel_id
                    || channel_join_confirm.channel_id != channel_join_confirm.requested_channel_id
                    || channel_join_confirm.channel_id != channel_id
                {
                    return Err(general_err!("received bad MCS Channel Join Confirm"));
                }

                let next_index = index + 1;

                let next_state = if next_index == self.channel_ids.len() {
                    ChannelConnectionState::AllJoined { user_channel_id }
                } else {
                    ChannelConnectionState::SendChannelJoinRequest {
                        user_channel_id,
                        index: next_index,
                    }
                };

                (Written::Nothing, next_state)
            }

            ChannelConnectionState::AllJoined { .. } => return Err(general_err!("all channels are already joined")),
        };

        self.state = next_state;

        Ok(written)
    }

    fn state(&self) -> &dyn State {
        &self.state
    }
}
