use std::collections::HashSet;
use std::mem;

use ironrdp_pdu::write_buf::WriteBuf;
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
    },
    WaitChannelJoinConfirm {
        user_channel_id: u16,
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
    pub channel_ids: HashSet<u16>,
}

impl ChannelConnectionSequence {
    pub fn new(io_channel_id: u16, channel_ids: Vec<u16>) -> Self {
        let mut channel_ids: HashSet<u16> = channel_ids.into_iter().collect();

        // I/O channel ID must be joined as well.
        channel_ids.insert(io_channel_id);

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

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
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

                // User channel ID must also be joined.
                self.channel_ids.insert(user_channel_id);

                debug!(message = ?attach_user_confirm, user_channel_id, "Received");

                debug_assert!(!self.channel_ids.is_empty());

                (
                    Written::Nothing,
                    ChannelConnectionState::SendChannelJoinRequest { user_channel_id },
                )
            }

            // Send all the join requests in a single batch.
            // > RDP 4.0, 5.0, 5.1, 5.2, 6.0, 6.1, 7.0, 7.1, 8.0, 10.2, 10.3,
            // > 10.4, and 10.5 clients send a Channel Join Request to the server only after the
            // > Channel Join Confirm for a previously sent request has been received. RDP 8.1,
            // > 10.0, and 10.1 clients send all of the Channel Join Requests to the server in a
            // > single batch to minimize the overall connection sequence time.
            ChannelConnectionState::SendChannelJoinRequest { user_channel_id } => {
                let mut written = 0;

                for channel_id in self.channel_ids.iter().copied() {
                    let channel_join_request = mcs::ChannelJoinRequest {
                        initiator_id: user_channel_id,
                        channel_id,
                    };

                    debug!(message = ?channel_join_request, "Send");

                    written += ironrdp_pdu::encode_buf(&channel_join_request, output).map_err(ConnectorError::pdu)?;
                }

                (
                    Written::from_size(written)?,
                    ChannelConnectionState::WaitChannelJoinConfirm { user_channel_id },
                )
            }

            ChannelConnectionState::WaitChannelJoinConfirm { user_channel_id } => {
                let channel_join_confirm =
                    ironrdp_pdu::decode::<mcs::ChannelJoinConfirm>(input).map_err(ConnectorError::pdu)?;

                debug!(message = ?channel_join_confirm, "Received");

                if channel_join_confirm.initiator_id != user_channel_id {
                    warn!(
                        channel_join_confirm.initiator_id,
                        user_channel_id, "Inconsistent initiator ID for MCS Channel Join Confirm",
                    )
                }

                let is_expected = self.channel_ids.remove(&channel_join_confirm.requested_channel_id);

                if !is_expected {
                    return Err(reason_err!(
                        "ChannelJoinConfirm",
                        "unexpected requested_channel_id in MCS Channel Join Confirm: got {}, expected one of: {:?}",
                        channel_join_confirm.requested_channel_id,
                        self.channel_ids,
                    ));
                }

                if channel_join_confirm.requested_channel_id != channel_join_confirm.channel_id {
                    // We could handle that gracefully by updating the StaticChannelSet, but it doesn’t seem to ever happen.
                    return Err(reason_err!(
                        "ChannelJoinConfirm",
                        "a channel was joined with a different channel ID than requested: requested {}, got {}",
                        channel_join_confirm.requested_channel_id,
                        channel_join_confirm.channel_id,
                    ));
                }

                let next_state = if self.channel_ids.is_empty() {
                    ChannelConnectionState::AllJoined { user_channel_id }
                } else {
                    ChannelConnectionState::WaitChannelJoinConfirm { user_channel_id }
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
