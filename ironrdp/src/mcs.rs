mod connect_initial;
#[cfg(test)]
mod test;

pub use self::connect_initial::{ConnectInitial, ConnectResponse, DomainParameters};

use std::io;

use byteorder::{ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{gcc::GccError, impl_from_error, per, PduParsing};

pub const RESULT_ENUM_LENGTH: u8 = 16;

const BASE_CHANNEL_ID: u16 = 1001;
const SEND_DATA_PDU_DATA_PRIORITY_AND_SEGMENTATION: u8 = 0x70;

/// The kind of the RDP header message that may carry additional data.
#[derive(Debug, Clone, PartialEq)]
pub enum McsPdu {
    ErectDomainRequest(ErectDomainPdu),
    AttachUserRequest,
    AttachUserConfirm(AttachUserConfirmPdu),
    ChannelJoinRequest(ChannelJoinRequestPdu),
    ChannelJoinConfirm(ChannelJoinConfirmPdu),
    SendDataRequest(SendDataContext),
    SendDataIndication(SendDataContext),
    DisconnectProviderUltimatum(DisconnectUltimatumReason),
}

impl McsPdu {
    pub fn as_short_name(&self) -> &str {
        match self {
            McsPdu::ErectDomainRequest(_) => "Erect Domain Request",
            McsPdu::AttachUserRequest => "Attach User Request",
            McsPdu::AttachUserConfirm(_) => "Attach User Confirm",
            McsPdu::ChannelJoinRequest(_) => "Channel Join Request",
            McsPdu::ChannelJoinConfirm(_) => "Channel Join Confirm",
            McsPdu::SendDataRequest(_) => "Send Data Context",
            McsPdu::SendDataIndication(_) => "Send Data Indication",
            McsPdu::DisconnectProviderUltimatum(_) => "Disconnect Provider Ultimatum",
        }
    }
}

impl PduParsing for McsPdu {
    type Error = McsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let choice = per::read_choice(&mut stream)?;
        let mcs_pdu = DomainMcsPdu::from_u8(choice >> 2).ok_or(McsError::InvalidDomainMcsPdu)?;

        match mcs_pdu {
            DomainMcsPdu::ErectDomainRequest => Ok(McsPdu::ErectDomainRequest(
                ErectDomainPdu::from_buffer(&mut stream)?,
            )),
            DomainMcsPdu::AttachUserRequest => Ok(McsPdu::AttachUserRequest),
            DomainMcsPdu::AttachUserConfirm => Ok(McsPdu::AttachUserConfirm(
                AttachUserConfirmPdu::from_buffer(&mut stream)?,
            )),
            DomainMcsPdu::ChannelJoinRequest => Ok(McsPdu::ChannelJoinRequest(
                ChannelJoinRequestPdu::from_buffer(&mut stream)?,
            )),
            DomainMcsPdu::ChannelJoinConfirm => Ok(McsPdu::ChannelJoinConfirm(
                ChannelJoinConfirmPdu::from_buffer(&mut stream)?,
            )),
            DomainMcsPdu::DisconnectProviderUltimatum => Ok(McsPdu::DisconnectProviderUltimatum(
                DisconnectUltimatumReason::from_choice(&mut stream, choice)?,
            )),
            DomainMcsPdu::SendDataRequest => Ok(McsPdu::SendDataRequest(
                SendDataContext::from_buffer(&mut stream)?,
            )),
            DomainMcsPdu::SendDataIndication => Ok(McsPdu::SendDataIndication(
                SendDataContext::from_buffer(&mut stream)?,
            )),
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let (domain_mcs_pdu, options) = match self {
            McsPdu::ErectDomainRequest(_) => (DomainMcsPdu::ErectDomainRequest, 0),
            McsPdu::AttachUserRequest => (DomainMcsPdu::AttachUserRequest, 0),
            McsPdu::AttachUserConfirm(_) => (DomainMcsPdu::AttachUserConfirm, 2),
            McsPdu::ChannelJoinRequest(_) => (DomainMcsPdu::ChannelJoinRequest, 0),
            McsPdu::ChannelJoinConfirm(_) => (DomainMcsPdu::ChannelJoinConfirm, 2),
            McsPdu::DisconnectProviderUltimatum(reason) => {
                (DomainMcsPdu::DisconnectProviderUltimatum, reason.options())
            }
            McsPdu::SendDataRequest(_) => (DomainMcsPdu::SendDataRequest, 0),
            McsPdu::SendDataIndication(_) => (DomainMcsPdu::SendDataIndication, 0),
        };
        per::write_choice(
            &mut stream,
            (domain_mcs_pdu.to_u8().unwrap() << 2) | options,
        )?;

        match self {
            McsPdu::ErectDomainRequest(erect_domain_request) => {
                erect_domain_request.to_buffer(&mut stream)?
            }
            McsPdu::AttachUserRequest => (),
            McsPdu::AttachUserConfirm(attach_user_confirm_pdu) => {
                attach_user_confirm_pdu.to_buffer(&mut stream)?
            }
            McsPdu::ChannelJoinRequest(channel_join_request_pdu) => {
                channel_join_request_pdu.to_buffer(&mut stream)?
            }
            McsPdu::ChannelJoinConfirm(channel_join_confirm_pdu) => {
                channel_join_confirm_pdu.to_buffer(&mut stream)?
            }
            McsPdu::DisconnectProviderUltimatum(reason) => reason.to_buffer(&mut stream)?,
            McsPdu::SendDataRequest(send_data) => send_data.to_buffer(&mut stream)?,
            McsPdu::SendDataIndication(send_data) => send_data.to_buffer(&mut stream)?,
        };

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let pdu_length = match self {
            McsPdu::ErectDomainRequest(erect_domain_request) => {
                erect_domain_request.buffer_length()
            }
            McsPdu::AttachUserRequest => 0,
            McsPdu::AttachUserConfirm(attach_user_confirm_pdu) => {
                attach_user_confirm_pdu.buffer_length()
            }
            McsPdu::ChannelJoinRequest(channel_join_request_pdu) => {
                channel_join_request_pdu.buffer_length()
            }
            McsPdu::ChannelJoinConfirm(channel_join_confirm_pdu) => {
                channel_join_confirm_pdu.buffer_length()
            }
            McsPdu::DisconnectProviderUltimatum(reason) => reason.buffer_length(),
            McsPdu::SendDataRequest(send_data) => send_data.buffer_length(),
            McsPdu::SendDataIndication(send_data) => send_data.buffer_length(),
        };

        per::SIZEOF_CHOICE + pdu_length
    }
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
enum DomainMcsPdu {
    ErectDomainRequest = 1,
    DisconnectProviderUltimatum = 8,
    AttachUserRequest = 10,
    AttachUserConfirm = 11,
    ChannelJoinRequest = 14,
    ChannelJoinConfirm = 15,
    SendDataRequest = 25,
    SendDataIndication = 26,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErectDomainPdu {
    pub sub_height: u32,
    pub sub_interval: u32,
}

impl PduParsing for ErectDomainPdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let sub_height = per::read_u32(&mut stream)?;
        let sub_interval = per::read_u32(&mut stream)?;

        Ok(Self {
            sub_height,
            sub_interval,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        per::write_u32(&mut stream, self.sub_height)?;
        per::write_u32(&mut stream, self.sub_interval)?;

        Ok(())
    }
    fn buffer_length(&self) -> usize {
        per::sizeof_u32(self.sub_height) + per::sizeof_u32(self.sub_interval)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttachUserConfirmPdu {
    pub initiator_id: u16,
    pub result: u8,
}

impl PduParsing for AttachUserConfirmPdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let result = per::read_enum(&mut stream, RESULT_ENUM_LENGTH)?;
        let user_id = per::read_u16(&mut stream, BASE_CHANNEL_ID)?;

        Ok(Self {
            result,
            initiator_id: user_id,
        })
    }
    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        per::write_enum(&mut stream, self.result)?;
        per::write_u16(&mut stream, self.initiator_id, BASE_CHANNEL_ID)?;

        Ok(())
    }
    fn buffer_length(&self) -> usize {
        per::SIZEOF_ENUM + per::SIZEOF_U16
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelJoinRequestPdu {
    pub initiator_id: u16,
    pub channel_id: u16,
}

impl PduParsing for ChannelJoinRequestPdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let user_id = per::read_u16(&mut stream, BASE_CHANNEL_ID)?;
        let channel_id = per::read_u16(&mut stream, 0)?;

        Ok(Self {
            initiator_id: user_id,
            channel_id,
        })
    }
    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        per::write_u16(&mut stream, self.initiator_id, BASE_CHANNEL_ID)?;
        per::write_u16(&mut stream, self.channel_id, 0)?;

        Ok(())
    }
    fn buffer_length(&self) -> usize {
        per::SIZEOF_U16 * 2
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelJoinConfirmPdu {
    pub channel_id: u16,
    pub result: u8,
    pub initiator_id: u16,
    pub requested_channel_id: u16,
}

impl PduParsing for ChannelJoinConfirmPdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let result = per::read_enum(&mut stream, RESULT_ENUM_LENGTH)?;
        let initiator_id = per::read_u16(&mut stream, BASE_CHANNEL_ID)?;
        let requested_channel_id = per::read_u16(&mut stream, 0)?;
        let channel_id = per::read_u16(&mut stream, 0)?;

        Ok(Self {
            result,
            initiator_id,
            requested_channel_id,
            channel_id,
        })
    }
    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        per::write_enum(&mut stream, self.result)?;
        per::write_u16(&mut stream, self.initiator_id, BASE_CHANNEL_ID)?;
        per::write_u16(&mut stream, self.requested_channel_id, 0)?;
        per::write_u16(&mut stream, self.channel_id, 0)?;

        Ok(())
    }
    fn buffer_length(&self) -> usize {
        per::SIZEOF_ENUM + per::SIZEOF_U16 * 3
    }
}

/// Contains the channel ID and the length of the data. This structure is a part of the
/// [`RdpHeaderMessage`](enum.RdpHeaderMessage.html).
#[derive(Debug, Clone, PartialEq)]
pub struct SendDataContext {
    pub initiator_id: u16,
    pub channel_id: u16,
    pub pdu_length: usize,
}

impl PduParsing for SendDataContext {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let initiator_id = per::read_u16(&mut stream, BASE_CHANNEL_ID)?;
        let channel_id = per::read_u16(&mut stream, 0)?;
        let _data_priority_and_segmentation = stream.read_u8()?;
        let (pdu_length, _) = per::read_length(&mut stream)?;

        Ok(Self {
            initiator_id,
            channel_id,
            pdu_length: pdu_length as usize,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        per::write_u16(&mut stream, self.initiator_id, BASE_CHANNEL_ID)?;
        per::write_u16(&mut stream, self.channel_id, 0)?;
        stream.write_u8(SEND_DATA_PDU_DATA_PRIORITY_AND_SEGMENTATION)?;
        per::write_length(&mut stream, self.pdu_length as u16)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        per::SIZEOF_U16 * 2 + 1 + per::sizeof_length(self.pdu_length as u16)
    }
}

/// The reason of [`DisconnectProviderUltimatum`](enum.RdpHeaderMessage.html).
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum DisconnectUltimatumReason {
    DomainDisconnected = 0,
    ProviderInitiated = 1,
    TokenPurged = 2,
    UserRequested = 3,
    ChannelPurged = 4,
}

impl DisconnectUltimatumReason {
    fn from_choice(mut stream: impl io::Read, choice: u8) -> Result<Self, McsError> {
        let b = per::read_choice(&mut stream)?;

        Self::from_u8(((choice & 0x01) << 1) | (b >> 7))
            .ok_or(McsError::InvalidDisconnectProviderUltimatum)
    }

    fn to_buffer(self, mut stream: impl io::Write) -> Result<(), McsError> {
        let enumerated = match self {
            DisconnectUltimatumReason::UserRequested
            | DisconnectUltimatumReason::ProviderInitiated => 0x80,
            _ => 0x40,
        };
        per::write_enum(&mut stream, enumerated)?;

        Ok(())
    }
    fn buffer_length(self) -> usize {
        per::SIZEOF_CHOICE
    }

    fn options(self) -> u8 {
        match self {
            DisconnectUltimatumReason::TokenPurged | DisconnectUltimatumReason::UserRequested => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Fail)]
pub enum McsError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "GCC block error: {}", _0)]
    GccError(#[fail(cause)] GccError),
    #[fail(display = "Invalid disconnect provider ultimatum")]
    InvalidDisconnectProviderUltimatum,
    #[fail(display = "Invalid domain MCS PDU")]
    InvalidDomainMcsPdu,
    #[fail(display = "Invalid MCS Connection Sequence PDU: {}", _0)]
    InvalidPdu(String),
    #[fail(display = "Invalid invalid MCS channel id: {}", _0)]
    UnexpectedChannelId(String),
}

impl_from_error!(io::Error, McsError, McsError::IOError);
impl_from_error!(GccError, McsError, McsError::GccError);

impl From<McsError> for io::Error {
    fn from(e: McsError) -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            format!("MCS Connection Sequence error: {}", e),
        )
    }
}
