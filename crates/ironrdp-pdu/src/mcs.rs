use std::borrow::Cow;

use crate::gcc::{ChannelDef, ClientGccBlocks, ConferenceCreateRequest, ConferenceCreateResponse};
use crate::tpdu::{TpduCode, TpduHeader};
use crate::tpkt::TpktHeader;
use crate::x224::{user_data_size, X224Pdu};
use crate::{per, PduError, PduErrorExt as _, PduResult};
use ironrdp_core::{IntoOwned, ReadCursor, WriteCursor};

// T.125 MCS is defined in:
//
// http://www.itu.int/rec/T-REC-T.125-199802-I/
// ITU-T T.125 Multipoint Communication Service Protocol Specification
//
// Connect-Initial ::= [APPLICATION 101] IMPLICIT SEQUENCE
// {
//     callingDomainSelector	OCTET_STRING,
//     calledDomainSelector		OCTET_STRING,
//     upwardFlag			    BOOLEAN,
//     targetParameters		    DomainParameters,
//     minimumParameters		DomainParameters,
//     maximumParameters		DomainParameters,
//     userData			        OCTET_STRING
// }
//
// DomainParameters ::= SEQUENCE
// {
//     maxChannelIds		INTEGER (0..MAX),
//     maxUserIds			INTEGER (0..MAX),
//     maxTokenIds			INTEGER (0..MAX),
//     numPriorities		INTEGER (0..MAX),
//     minThroughput		INTEGER (0..MAX),
//     maxHeight			INTEGER (0..MAX),
//     maxMCSPDUsize		INTEGER (0..MAX),
//     protocolVersion		INTEGER (0..MAX)
// }
//
// Connect-Response ::= [APPLICATION 102] IMPLICIT SEQUENCE
// {
//     result				Result,
//     calledConnectId		INTEGER (0..MAX),
//     domainParameters		DomainParameters,
//     userData			    OCTET_STRING
// }
//
// Result ::= ENUMERATED
// {
//     rt-successful			    (0),
//     rt-domain-merging		    (1),
//     rt-domain-not-hierarchical	(2),
//     rt-no-such-channel		    (3),
//     rt-no-such-domain		    (4),
//     rt-no-such-user			    (5),
//     rt-not-admitted			    (6),
//     rt-other-user-id		        (7),
//     rt-parameters-unacceptable	(8),
//     rt-token-not-available		(9),
//     rt-token-not-possessed		(10),
//     rt-too-many-channels		    (11),
//     rt-too-many-tokens		    (12),
//     rt-too-many-users		    (13),
//     rt-unspecified-failure		(14),
//     rt-user-rejected		        (15)
// }
//
// ErectDomainRequest ::= [APPLICATION 1] IMPLICIT SEQUENCE
// {
//     subHeight		INTEGER (0..MAX),
//     subInterval		INTEGER (0..MAX)
// }
//
// AttachUserRequest ::= [APPLICATION 10] IMPLICIT SEQUENCE
// {
// }
//
// AttachUserConfirm ::= [APPLICATION 11] IMPLICIT SEQUENCE
// {
//     result			Result,
//     initiator		UserId OPTIONAL
// }
//
// ChannelJoinRequest ::= [APPLICATION 14] IMPLICIT SEQUENCE
// {
//     initiator		UserId,
//     channelId		ChannelId
// }
//
// ChannelJoinConfirm ::= [APPLICATION 15] IMPLICIT SEQUENCE
// {
//     result		Result,
//     initiator	UserId,
//     requested	ChannelId,
//     channelId	ChannelId OPTIONAL
// }
//
// SendDataRequest ::= [APPLICATION 25] IMPLICIT SEQUENCE
// {
//     initiator		UserId,
//     channelId		ChannelId,
//     dataPriority		DataPriority,
//     segmentation		Segmentation,
//     userData			OCTET_STRING
// }
//
// DataPriority ::= CHOICE
// {
//     top		NULL,
//     high		NULL,
//     medium	NULL,
//     low		NULL,
//     ...
// }
//
// Segmentation ::= BIT_STRING
// {
//     begin	(0),
//     end		(1)
// } (SIZE(2))
//
// SendDataIndication ::= [APPLICATION 26] IMPLICIT SEQUENCE
// {
//     initiator		UserId,
//     channelId		ChannelId,
//     dataPriority		DataPriority,
//     segmentation		Segmentation,
//     userData			OCTET_STRING
// }

pub const RESULT_ENUM_LENGTH: u8 = 16;

const BASE_CHANNEL_ID: u16 = 1001;
const SEND_DATA_PDU_DATA_PRIORITY_AND_SEGMENTATION: u8 = 0x70;

/// Creates a closure mapping a `PerError` to a `PduError` with field-level context.
///
/// Shorthand for
/// ```rust
/// |e| <crate::PduError as crate::PduErrorExt>::invalid_message(Self::MCS_NAME, field_name, "PER").with_source(e)
/// ```
macro_rules! per_field_err {
    ($field_name:expr) => {{
        |error| {
            <$crate::PduError as $crate::PduErrorExt>::invalid_message(Self::MCS_NAME, $field_name, "PER")
                .with_source(error)
        }
    }};
}

#[doc(hidden)]
pub trait McsPdu<'de>: Sized {
    const MCS_NAME: &'static str;

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()>;

    fn mcs_body_decode(src: &mut ReadCursor<'de>, tpdu_user_data_size: usize) -> PduResult<Self>;

    fn mcs_size(&self) -> usize;

    fn name(&self) -> &'static str {
        Self::MCS_NAME
    }
}

impl<'de, T> X224Pdu<'de> for T
where
    T: McsPdu<'de>,
{
    const X224_NAME: &'static str = T::MCS_NAME;

    const TPDU_CODE: TpduCode = TpduCode::DATA;

    fn x224_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.mcs_body_encode(dst)
    }

    fn x224_body_decode(src: &mut ReadCursor<'de>, tpkt: &TpktHeader, tpdu: &TpduHeader) -> PduResult<Self> {
        let tpdu_user_data_size = user_data_size(tpkt, tpdu);
        T::mcs_body_decode(src, tpdu_user_data_size)
    }

    fn tpdu_header_variable_part_size(&self) -> usize {
        0
    }

    fn tpdu_user_data_size(&self) -> usize {
        self.mcs_size()
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(u8)]
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

impl DomainMcsPdu {
    fn check_expected(self, name: &'static str, expected: DomainMcsPdu) -> PduResult<()> {
        if self != expected {
            Err(PduError::unexpected_message_type(name, self.as_u8()))
        } else {
            Ok(())
        }
    }

    fn from_choice(choice: u8) -> Option<Self> {
        Self::from_u8(choice >> 2)
    }

    fn to_choice(self) -> u8 {
        self.as_u8() << 2
    }

    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::ErectDomainRequest),
            8 => Some(Self::DisconnectProviderUltimatum),
            10 => Some(Self::AttachUserRequest),
            11 => Some(Self::AttachUserConfirm),
            14 => Some(Self::ChannelJoinRequest),
            15 => Some(Self::ChannelJoinConfirm),
            25 => Some(Self::SendDataRequest),
            26 => Some(Self::SendDataIndication),
            _ => None,
        }
    }

    fn as_u8(self) -> u8 {
        self as u8
    }
}

fn read_mcspdu_header(src: &mut ReadCursor<'_>, ctx: &'static str) -> PduResult<DomainMcsPdu> {
    let choice = src.try_read_u8().map_err(|e| custom_err!(ctx, e))?;

    DomainMcsPdu::from_choice(choice)
        .ok_or_else(|| PduError::invalid_message(ctx, "domain-mcspdu", "unexpected application tag for CHOICE"))
}

fn peek_mcspdu_header(src: &mut ReadCursor<'_>, ctx: &'static str) -> PduResult<DomainMcsPdu> {
    let choice = src.try_read_u8().map_err(|e| custom_err!(ctx, e))?;

    DomainMcsPdu::from_choice(choice)
        .ok_or_else(|| PduError::invalid_message(ctx, "domain-mcspdu", "unexpected application tag for CHOICE"))
}

fn write_mcspdu_header(dst: &mut WriteCursor<'_>, domain_mcspdu: DomainMcsPdu, options: u8) {
    let choice = domain_mcspdu.to_choice();

    debug_assert_eq!(options & !0b11, 0);
    debug_assert_eq!(choice & 0b11, 0);

    dst.write_u8(choice | options);
}

/// The kind of the RDP header message that may carry additional data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McsMessage<'a> {
    ErectDomainRequest(ErectDomainPdu),
    AttachUserRequest(AttachUserRequest),
    AttachUserConfirm(AttachUserConfirm),
    ChannelJoinRequest(ChannelJoinRequest),
    ChannelJoinConfirm(ChannelJoinConfirm),
    SendDataRequest(SendDataRequest<'a>),
    SendDataIndication(SendDataIndication<'a>),
    DisconnectProviderUltimatum(DisconnectProviderUltimatum),
}

impl_pdu_borrowing!(McsMessage<'_>, OwnedMcsMessage);

impl IntoOwned for McsMessage<'_> {
    type Owned = OwnedMcsMessage;

    fn into_owned(self) -> Self::Owned {
        match self {
            Self::ErectDomainRequest(msg) => McsMessage::ErectDomainRequest(msg.into_owned()),
            Self::AttachUserRequest(msg) => McsMessage::AttachUserRequest(msg.into_owned()),
            Self::AttachUserConfirm(msg) => McsMessage::AttachUserConfirm(msg.into_owned()),
            Self::ChannelJoinRequest(msg) => McsMessage::ChannelJoinRequest(msg.into_owned()),
            Self::ChannelJoinConfirm(msg) => McsMessage::ChannelJoinConfirm(msg.into_owned()),
            Self::SendDataRequest(msg) => McsMessage::SendDataRequest(msg.into_owned()),
            Self::SendDataIndication(msg) => McsMessage::SendDataIndication(msg.into_owned()),
            Self::DisconnectProviderUltimatum(msg) => McsMessage::DisconnectProviderUltimatum(msg.into_owned()),
        }
    }
}

impl<'de> McsPdu<'de> for McsMessage<'de> {
    const MCS_NAME: &'static str = "McsMessage";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            Self::ErectDomainRequest(msg) => msg.mcs_body_encode(dst),
            Self::AttachUserRequest(msg) => msg.mcs_body_encode(dst),
            Self::AttachUserConfirm(msg) => msg.mcs_body_encode(dst),
            Self::ChannelJoinRequest(msg) => msg.mcs_body_encode(dst),
            Self::ChannelJoinConfirm(msg) => msg.mcs_body_encode(dst),
            Self::SendDataRequest(msg) => msg.mcs_body_encode(dst),
            Self::SendDataIndication(msg) => msg.mcs_body_encode(dst),
            Self::DisconnectProviderUltimatum(msg) => msg.mcs_body_encode(dst),
        }
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, tpdu_user_data_size: usize) -> PduResult<Self> {
        match peek_mcspdu_header(src, Self::MCS_NAME)? {
            DomainMcsPdu::ErectDomainRequest => Ok(McsMessage::ErectDomainRequest(ErectDomainPdu::mcs_body_decode(
                src,
                tpdu_user_data_size,
            )?)),
            DomainMcsPdu::AttachUserRequest => Ok(McsMessage::AttachUserRequest(AttachUserRequest::mcs_body_decode(
                src,
                tpdu_user_data_size,
            )?)),
            DomainMcsPdu::AttachUserConfirm => Ok(McsMessage::AttachUserConfirm(AttachUserConfirm::mcs_body_decode(
                src,
                tpdu_user_data_size,
            )?)),
            DomainMcsPdu::ChannelJoinRequest => Ok(McsMessage::ChannelJoinRequest(
                ChannelJoinRequest::mcs_body_decode(src, tpdu_user_data_size)?,
            )),
            DomainMcsPdu::ChannelJoinConfirm => Ok(McsMessage::ChannelJoinConfirm(
                ChannelJoinConfirm::mcs_body_decode(src, tpdu_user_data_size)?,
            )),
            DomainMcsPdu::SendDataRequest => Ok(McsMessage::SendDataRequest(SendDataRequest::mcs_body_decode(
                src,
                tpdu_user_data_size,
            )?)),
            DomainMcsPdu::SendDataIndication => Ok(McsMessage::SendDataIndication(
                SendDataIndication::mcs_body_decode(src, tpdu_user_data_size)?,
            )),
            DomainMcsPdu::DisconnectProviderUltimatum => Ok(McsMessage::DisconnectProviderUltimatum(
                DisconnectProviderUltimatum::mcs_body_decode(src, tpdu_user_data_size)?,
            )),
        }
    }

    fn mcs_size(&self) -> usize {
        match self {
            Self::ErectDomainRequest(msg) => msg.mcs_size(),
            Self::AttachUserRequest(msg) => msg.mcs_size(),
            Self::AttachUserConfirm(msg) => msg.mcs_size(),
            Self::ChannelJoinRequest(msg) => msg.mcs_size(),
            Self::ChannelJoinConfirm(msg) => msg.mcs_size(),
            Self::SendDataRequest(msg) => msg.mcs_size(),
            Self::SendDataIndication(msg) => msg.mcs_size(),
            Self::DisconnectProviderUltimatum(msg) => msg.mcs_size(),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::ErectDomainRequest(msg) => msg.name(),
            Self::AttachUserRequest(msg) => msg.name(),
            Self::AttachUserConfirm(msg) => msg.name(),
            Self::ChannelJoinRequest(msg) => msg.name(),
            Self::ChannelJoinConfirm(msg) => msg.name(),
            Self::SendDataRequest(msg) => msg.name(),
            Self::SendDataIndication(msg) => msg.name(),
            Self::DisconnectProviderUltimatum(msg) => msg.name(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErectDomainPdu {
    pub sub_height: u32,
    pub sub_interval: u32,
}

impl_pdu_pod!(ErectDomainPdu);

impl<'de> McsPdu<'de> for ErectDomainPdu {
    const MCS_NAME: &'static str = "ErectDomainPdu";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_mcspdu_header(dst, DomainMcsPdu::ErectDomainRequest, 0);

        per::write_u32(dst, self.sub_height);
        per::write_u32(dst, self.sub_interval);

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, _: usize) -> PduResult<Self> {
        read_mcspdu_header(src, Self::MCS_NAME)?.check_expected(Self::MCS_NAME, DomainMcsPdu::ErectDomainRequest)?;

        let sub_height = per::read_u32(src).map_err(per_field_err!("subHeight"))?;
        let sub_interval = per::read_u32(src).map_err(per_field_err!("subInterval"))?;

        Ok(Self {
            sub_height,
            sub_interval,
        })
    }

    fn mcs_size(&self) -> usize {
        per::CHOICE_SIZE + per::sizeof_u32(self.sub_height) + per::sizeof_u32(self.sub_interval)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachUserRequest;

impl_pdu_pod!(AttachUserRequest);

impl<'de> McsPdu<'de> for AttachUserRequest {
    const MCS_NAME: &'static str = "AttachUserRequest";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_mcspdu_header(dst, DomainMcsPdu::AttachUserRequest, 0);

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, _: usize) -> PduResult<Self> {
        read_mcspdu_header(src, Self::MCS_NAME)?.check_expected(Self::MCS_NAME, DomainMcsPdu::AttachUserRequest)?;

        Ok(Self)
    }

    fn mcs_size(&self) -> usize {
        per::CHOICE_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachUserConfirm {
    pub result: u8,
    pub initiator_id: u16,
}

impl_pdu_pod!(AttachUserConfirm);

impl<'de> McsPdu<'de> for AttachUserConfirm {
    const MCS_NAME: &'static str = "AttachUserConfirm";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_mcspdu_header(dst, DomainMcsPdu::AttachUserConfirm, 2);

        per::write_enum(dst, self.result);
        per::write_u16(dst, self.initiator_id, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, _: usize) -> PduResult<Self> {
        read_mcspdu_header(src, Self::MCS_NAME)?.check_expected(Self::MCS_NAME, DomainMcsPdu::AttachUserConfirm)?;

        let result = per::read_enum(src, RESULT_ENUM_LENGTH).map_err(per_field_err!("result"))?;
        let user_id = per::read_u16(src, BASE_CHANNEL_ID).map_err(per_field_err!("userId"))?;

        Ok(Self {
            result,
            initiator_id: user_id,
        })
    }

    fn mcs_size(&self) -> usize {
        per::CHOICE_SIZE + per::ENUM_SIZE + per::U16_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelJoinRequest {
    pub initiator_id: u16,
    pub channel_id: u16,
}

impl_pdu_pod!(ChannelJoinRequest);

impl<'de> McsPdu<'de> for ChannelJoinRequest {
    const MCS_NAME: &'static str = "ChannelJoinRequest";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_mcspdu_header(dst, DomainMcsPdu::ChannelJoinRequest, 0);

        per::write_u16(dst, self.initiator_id, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        per::write_u16(dst, self.channel_id, 0).map_err(per_field_err!("channelId"))?;

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, _: usize) -> PduResult<Self> {
        read_mcspdu_header(src, Self::MCS_NAME)?.check_expected(Self::MCS_NAME, DomainMcsPdu::ChannelJoinRequest)?;

        let initiator_id = per::read_u16(src, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        let channel_id = per::read_u16(src, 0).map_err(per_field_err!("channelID"))?;

        Ok(Self {
            initiator_id,
            channel_id,
        })
    }

    fn mcs_size(&self) -> usize {
        per::CHOICE_SIZE + per::U16_SIZE * 2
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelJoinConfirm {
    pub result: u8,
    pub initiator_id: u16,
    pub requested_channel_id: u16,
    pub channel_id: u16,
}

impl_pdu_pod!(ChannelJoinConfirm);

impl<'de> McsPdu<'de> for ChannelJoinConfirm {
    const MCS_NAME: &'static str = "ChannelJoinConfirm";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_mcspdu_header(dst, DomainMcsPdu::ChannelJoinConfirm, 2);

        per::write_enum(dst, self.result);
        per::write_u16(dst, self.initiator_id, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        per::write_u16(dst, self.requested_channel_id, 0).map_err(per_field_err!("requested"))?;
        per::write_u16(dst, self.channel_id, 0).map_err(per_field_err!("channelId"))?;

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, _: usize) -> PduResult<Self> {
        read_mcspdu_header(src, Self::MCS_NAME)?.check_expected(Self::MCS_NAME, DomainMcsPdu::ChannelJoinConfirm)?;

        let result = per::read_enum(src, RESULT_ENUM_LENGTH).map_err(per_field_err!("result"))?;
        let initiator_id = per::read_u16(src, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        let requested_channel_id = per::read_u16(src, 0).map_err(per_field_err!("requested"))?;
        let channel_id = per::read_u16(src, 0).map_err(per_field_err!("channelId"))?;

        Ok(Self {
            result,
            initiator_id,
            requested_channel_id,
            channel_id,
        })
    }

    fn mcs_size(&self) -> usize {
        per::CHOICE_SIZE + per::ENUM_SIZE + per::U16_SIZE * 3
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendDataRequest<'a> {
    pub initiator_id: u16,
    pub channel_id: u16,
    pub user_data: Cow<'a, [u8]>,
}

impl_pdu_borrowing!(SendDataRequest<'_>, OwnedSendDataRequest);

impl IntoOwned for SendDataRequest<'_> {
    type Owned = OwnedSendDataRequest;

    fn into_owned(self) -> Self::Owned {
        SendDataRequest {
            user_data: Cow::Owned(self.user_data.into_owned()),
            ..self
        }
    }
}

impl<'de> McsPdu<'de> for SendDataRequest<'de> {
    const MCS_NAME: &'static str = "SendDataRequest";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_mcspdu_header(dst, DomainMcsPdu::SendDataRequest, 0);

        per::write_u16(dst, self.initiator_id, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        per::write_u16(dst, self.channel_id, 0).map_err(per_field_err!("channelID"))?;

        dst.write_u8(SEND_DATA_PDU_DATA_PRIORITY_AND_SEGMENTATION);

        per::write_length(dst, cast_length!("user-data-length", self.user_data.len())?);
        dst.write_slice(&self.user_data);

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, tpdu_user_data_size: usize) -> PduResult<Self> {
        let src_len_before = src.len();

        read_mcspdu_header(src, Self::MCS_NAME)?.check_expected(Self::MCS_NAME, DomainMcsPdu::SendDataRequest)?;

        let initiator_id = per::read_u16(src, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        let channel_id = per::read_u16(src, 0).map_err(per_field_err!("channelId"))?;

        // dataPriority + segmentation
        ensure_size!(ctx: Self::MCS_NAME, in: src, size: 1);
        read_padding!(src, 1);

        let (length, _) = per::read_length(src).map_err(per_field_err!("userDataLength"))?;
        let length = usize::from(length);

        let src_len_after = src.len();

        if length > tpdu_user_data_size.saturating_sub(src_len_before - src_len_after) {
            return Err(PduError::invalid_message(
                Self::MCS_NAME,
                "userDataLength",
                "inconsistent with user data size advertised in TPDU",
            ));
        }

        ensure_size!(ctx: Self::MCS_NAME, in: src, size: length);
        let user_data = Cow::Borrowed(src.read_slice(length));

        Ok(Self {
            initiator_id,
            channel_id,
            user_data,
        })
    }

    fn mcs_size(&self) -> usize {
        per::CHOICE_SIZE
            + per::U16_SIZE * 2
            + 1
            + per::sizeof_length(u16::try_from(self.user_data.len()).unwrap_or(u16::MAX))
            + self.user_data.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendDataIndication<'a> {
    pub initiator_id: u16,
    pub channel_id: u16,
    pub user_data: Cow<'a, [u8]>,
}

impl_pdu_borrowing!(SendDataIndication<'_>, OwnedSendDataIndication);

impl IntoOwned for SendDataIndication<'_> {
    type Owned = OwnedSendDataIndication;

    fn into_owned(self) -> Self::Owned {
        SendDataIndication {
            user_data: Cow::Owned(self.user_data.into_owned()),
            ..self
        }
    }
}

impl<'de> McsPdu<'de> for SendDataIndication<'de> {
    const MCS_NAME: &'static str = "SendDataIndication";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_mcspdu_header(dst, DomainMcsPdu::SendDataIndication, 0);

        per::write_u16(dst, self.initiator_id, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        per::write_u16(dst, self.channel_id, 0).map_err(per_field_err!("channelId"))?;

        dst.write_u8(SEND_DATA_PDU_DATA_PRIORITY_AND_SEGMENTATION);

        per::write_length(dst, cast_length!("userDataLength", self.user_data.len())?);
        dst.write_slice(&self.user_data);

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, tpdu_user_data_size: usize) -> PduResult<Self> {
        let src_len_before = src.len();

        read_mcspdu_header(src, Self::MCS_NAME)?.check_expected(Self::MCS_NAME, DomainMcsPdu::SendDataIndication)?;

        let initiator_id = per::read_u16(src, BASE_CHANNEL_ID).map_err(per_field_err!("initiator"))?;
        let channel_id = per::read_u16(src, 0).map_err(per_field_err!("channelId"))?;

        // dataPriority + segmentation
        ensure_size!(ctx: Self::MCS_NAME, in: src, size: 1);
        read_padding!(src, 1);

        let (length, _) = per::read_length(src).map_err(per_field_err!("userDataLength"))?;
        let length = usize::from(length);

        let src_len_after = src.len();

        if length > tpdu_user_data_size.saturating_sub(src_len_before - src_len_after) {
            return Err(PduError::invalid_message(
                Self::MCS_NAME,
                "userDataLength",
                "inconsistent with user data size advertised in TPDU",
            ));
        }

        ensure_size!(ctx: Self::MCS_NAME, in: src, size: length);
        let user_data = Cow::Borrowed(src.read_slice(length));

        Ok(Self {
            initiator_id,
            channel_id,
            user_data,
        })
    }

    fn mcs_size(&self) -> usize {
        per::CHOICE_SIZE
            + per::U16_SIZE * 2
            + 1
            + per::sizeof_length(u16::try_from(self.user_data.len()).unwrap_or(u16::MAX))
            + self.user_data.len()
    }
}

/// The reason of `DisconnectProviderUltimatum`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum DisconnectReason {
    DomainDisconnected = 0,
    ProviderInitiated = 1,
    TokenPurged = 2,
    UserRequested = 3,
    ChannelPurged = 4,
}

impl DisconnectReason {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::DomainDisconnected),
            1 => Some(Self::ProviderInitiated),
            2 => Some(Self::TokenPurged),
            3 => Some(Self::UserRequested),
            4 => Some(Self::ChannelPurged),
            _ => None,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::DomainDisconnected => "domain disconnected",
            Self::ProviderInitiated => "server-initiated disconnect",
            Self::TokenPurged => "token purged",
            Self::UserRequested => "user-requested disconnect",
            Self::ChannelPurged => "channel purged",
        }
    }
}

impl core::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.description())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DisconnectProviderUltimatum {
    pub reason: DisconnectReason,
}

impl_pdu_pod!(DisconnectProviderUltimatum);

impl DisconnectProviderUltimatum {
    pub const NAME: &'static str = "DisconnectProviderUltimatum";

    pub const FIXED_PART_SIZE: usize = 2;

    pub fn from_reason(reason: DisconnectReason) -> Self {
        Self { reason }
    }
}

impl<'de> McsPdu<'de> for DisconnectProviderUltimatum {
    const MCS_NAME: &'static str = "DisconnectProviderUltimatum";

    fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let domain_mcspdu = DomainMcsPdu::DisconnectProviderUltimatum.as_u8();
        let reason = self.reason.as_u8();

        let b1 = domain_mcspdu << 2 | (reason >> 1 & 0x03);
        let b2 = reason << 7;

        dst.write_array([b1, b2]);

        Ok(())
    }

    fn mcs_body_decode(src: &mut ReadCursor<'de>, _: usize) -> PduResult<Self> {
        // http://msdn.microsoft.com/en-us/library/cc240872.aspx:
        //
        // PER encoded (ALIGNED variant of BASIC-PER) PDU contents:
        // 21 80
        //
        // 0x21:
        // 0 - --\
        // 0 -   |
        // 1 -   | CHOICE: From DomainMCSPDU select disconnectProviderUltimatum (8)
        // 0 -   | of type DisconnectProviderUltimatum
        // 0 -   |
        // 0 - --/
        // 0 - --\
        // 1 -   |
        //       | DisconnectProviderUltimatum::reason = rn-user-requested (3)
        // 0x80: |
        // 1 - --/
        // 0 - padding
        // 0 - padding
        // 0 - padding
        // 0 - padding
        // 0 - padding
        // 0 - padding
        // 0 - padding

        ensure_fixed_part_size!(in: src);

        let [b1, b2] = src.read_array();

        let domain_mcspdu_choice = b1 >> 2;
        let reason = (b1 & 0x03) << 1 | (b2 >> 7);

        DomainMcsPdu::from_u8(domain_mcspdu_choice)
            .ok_or_else(|| {
                PduError::invalid_message(Self::MCS_NAME, "domain-mcspdu", "unexpected application tag for CHOICE")
            })?
            .check_expected(Self::MCS_NAME, DomainMcsPdu::DisconnectProviderUltimatum)?;

        Ok(Self {
            reason: DisconnectReason::from_u8(reason)
                .ok_or_else(|| PduError::invalid_message(Self::MCS_NAME, "reason", "unknown variant"))?,
        })
    }

    fn mcs_size(&self) -> usize {
        2
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectInitial {
    pub conference_create_request: ConferenceCreateRequest,
    pub calling_domain_selector: Vec<u8>,
    pub called_domain_selector: Vec<u8>,
    pub upward_flag: bool,
    pub target_parameters: DomainParameters,
    pub min_parameters: DomainParameters,
    pub max_parameters: DomainParameters,
}

impl ConnectInitial {
    pub fn with_gcc_blocks(gcc_blocks: ClientGccBlocks) -> Self {
        Self {
            conference_create_request: ConferenceCreateRequest { gcc_blocks },
            calling_domain_selector: vec![0x01],
            called_domain_selector: vec![0x01],
            upward_flag: true,
            target_parameters: DomainParameters::target(),
            min_parameters: DomainParameters::min(),
            max_parameters: DomainParameters::max(),
        }
    }

    pub fn channel_names(&self) -> Option<Vec<ChannelDef>> {
        self.conference_create_request.gcc_blocks.channel_names()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectResponse {
    pub conference_create_response: ConferenceCreateResponse,
    pub called_connect_id: u32,
    pub domain_parameters: DomainParameters,
}

impl ConnectResponse {
    pub fn channel_ids(&self) -> Vec<u16> {
        self.conference_create_response.gcc_blocks.channel_ids()
    }

    pub fn global_channel_id(&self) -> u16 {
        self.conference_create_response.gcc_blocks.global_channel_id()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainParameters {
    pub max_channel_ids: u32,
    pub max_user_ids: u32,
    pub max_token_ids: u32,
    pub num_priorities: u32,
    pub min_throughput: u32,
    pub max_height: u32,
    pub max_mcs_pdu_size: u32,
    pub protocol_version: u32,
}

impl DomainParameters {
    pub fn min() -> Self {
        Self {
            max_channel_ids: 1,
            max_user_ids: 1,
            max_token_ids: 1,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 1056,
            protocol_version: 2,
        }
    }

    pub fn target() -> Self {
        Self {
            max_channel_ids: 34,
            max_user_ids: 2,
            max_token_ids: 0,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        }
    }

    pub fn max() -> Self {
        Self {
            max_channel_ids: 65535,
            max_user_ids: 64535,
            max_token_ids: 65535,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        }
    }
}

pub use legacy::McsError;

mod legacy {
    use std::io;

    use thiserror::Error;

    use super::*;
    use crate::gcc::conference_create::{ConferenceCreateRequest, ConferenceCreateResponse};
    use crate::gcc::GccError;
    use crate::{ber, PduDecode, PduEncode};

    // impl<'de> McsPdu<'de> for ConnectInitial {
    //     const MCS_NAME: &'static str = "DisconnectProviderUltimatum";

    //     fn mcs_body_encode(&self, dst: &mut WriteCursor<'_>) -> Result<()> {
    //         todo!()
    //     }

    //     fn mcs_body_decode(src: &mut ReadCursor<'de>, tpdu_user_data_size: usize) -> Result<Self> {
    //         todo!()
    //     }

    //     fn mcs_size(&self) -> usize {
    //         todo!()
    //     }
    // }

    const MCS_TYPE_CONNECT_INITIAL: u8 = 0x65;
    const MCS_TYPE_CONNECT_RESPONSE: u8 = 0x66;

    impl ConnectInitial {
        const NAME: &'static str = "ConnectInitial";

        fn fields_buffer_ber_length(&self) -> usize {
            ber::sizeof_octet_string(self.calling_domain_selector.len() as u16)
                + ber::sizeof_octet_string(self.called_domain_selector.len() as u16)
                + ber::SIZEOF_BOOL
                + (self.target_parameters.size() + self.min_parameters.size() + self.max_parameters.size())
                + ber::sizeof_octet_string(self.conference_create_request.size() as u16)
        }
    }

    impl PduEncode for ConnectInitial {
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(in: dst, size: self.size());

            ber::write_application_tag(dst, MCS_TYPE_CONNECT_INITIAL, self.fields_buffer_ber_length() as u16)?;
            ber::write_octet_string(dst, self.calling_domain_selector.as_ref())?;
            ber::write_octet_string(dst, self.called_domain_selector.as_ref())?;
            ber::write_bool(dst, self.upward_flag)?;
            self.target_parameters.encode(dst)?;
            self.min_parameters.encode(dst)?;
            self.max_parameters.encode(dst)?;
            ber::write_octet_string_tag(dst, cast_length!("len", self.conference_create_request.size())?)?;
            self.conference_create_request.encode(dst)?;

            Ok(())
        }

        fn name(&self) -> &'static str {
            Self::NAME
        }

        fn size(&self) -> usize {
            let fields_buffer_ber_length = self.fields_buffer_ber_length();

            fields_buffer_ber_length
                + ber::sizeof_application_tag(MCS_TYPE_CONNECT_INITIAL, fields_buffer_ber_length as u16)
        }
    }

    impl<'de> PduDecode<'de> for ConnectInitial {
        fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
            ber::read_application_tag(src, MCS_TYPE_CONNECT_INITIAL)?;
            let calling_domain_selector = ber::read_octet_string(src)?;
            let called_domain_selector = ber::read_octet_string(src)?;
            let upward_flag = ber::read_bool(src)?;
            let target_parameters = DomainParameters::decode(src)?;
            let min_parameters = DomainParameters::decode(src)?;
            let max_parameters = DomainParameters::decode(src)?;
            let _user_data_buffer_length = ber::read_octet_string_tag(src)?;
            let conference_create_request = ConferenceCreateRequest::decode(src)?;

            Ok(Self {
                conference_create_request,
                calling_domain_selector,
                called_domain_selector,
                upward_flag,
                target_parameters,
                min_parameters,
                max_parameters,
            })
        }
    }

    impl ConnectResponse {
        const NAME: &'static str = "ConnectResponse";

        fn fields_buffer_ber_length(&self) -> usize {
            ber::SIZEOF_ENUMERATED
                + ber::sizeof_integer(self.called_connect_id)
                + self.domain_parameters.size()
                + ber::sizeof_octet_string(self.conference_create_response.size() as u16)
        }
    }

    impl PduEncode for ConnectResponse {
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(in: dst, size: self.size());

            ber::write_application_tag(dst, MCS_TYPE_CONNECT_RESPONSE, self.fields_buffer_ber_length() as u16)?;
            ber::write_enumerated(dst, 0)?;
            ber::write_integer(dst, self.called_connect_id)?;
            self.domain_parameters.encode(dst)?;
            ber::write_octet_string_tag(dst, cast_length!("len", self.conference_create_response.size())?)?;
            self.conference_create_response.encode(dst)?;

            Ok(())
        }

        fn name(&self) -> &'static str {
            Self::NAME
        }

        fn size(&self) -> usize {
            let fields_buffer_ber_length = self.fields_buffer_ber_length();

            fields_buffer_ber_length
                + ber::sizeof_application_tag(MCS_TYPE_CONNECT_RESPONSE, fields_buffer_ber_length as u16)
        }
    }

    impl<'de> PduDecode<'de> for ConnectResponse {
        fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
            ber::read_application_tag(src, MCS_TYPE_CONNECT_RESPONSE)?;
            ber::read_enumerated(src, RESULT_ENUM_LENGTH)?;
            let called_connect_id = ber::read_integer(src)? as u32;
            let domain_parameters = DomainParameters::decode(src)?;
            let _user_data_buffer_length = ber::read_octet_string_tag(src)?;
            let conference_create_response = ConferenceCreateResponse::decode(src)?;

            Ok(Self {
                called_connect_id,
                domain_parameters,
                conference_create_response,
            })
        }
    }

    impl DomainParameters {
        const NAME: &'static str = "DomainParameters";

        fn fields_buffer_ber_length(&self) -> usize {
            ber::sizeof_integer(self.max_channel_ids)
                + ber::sizeof_integer(self.max_user_ids)
                + ber::sizeof_integer(self.max_token_ids)
                + ber::sizeof_integer(self.num_priorities)
                + ber::sizeof_integer(self.min_throughput)
                + ber::sizeof_integer(self.max_height)
                + ber::sizeof_integer(self.max_mcs_pdu_size)
                + ber::sizeof_integer(self.protocol_version)
        }
    }

    impl PduEncode for DomainParameters {
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(in: dst, size: self.size());

            ber::write_sequence_tag(dst, cast_length!("seqTagLen", self.fields_buffer_ber_length())?)?;
            ber::write_integer(dst, self.max_channel_ids)?;
            ber::write_integer(dst, self.max_user_ids)?;
            ber::write_integer(dst, self.max_token_ids)?;
            ber::write_integer(dst, self.num_priorities)?;
            ber::write_integer(dst, self.min_throughput)?;
            ber::write_integer(dst, self.max_height)?;
            ber::write_integer(dst, self.max_mcs_pdu_size)?;
            ber::write_integer(dst, self.protocol_version)?;

            Ok(())
        }

        fn name(&self) -> &'static str {
            Self::NAME
        }

        fn size(&self) -> usize {
            let fields_buffer_ber_length = self.fields_buffer_ber_length();

            // FIXME: maybe size should return PduResult...
            fields_buffer_ber_length + ber::sizeof_sequence_tag(fields_buffer_ber_length as u16)
        }
    }

    impl<'de> PduDecode<'de> for DomainParameters {
        fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
            ber::read_sequence_tag(src)?;
            let max_channel_ids = ber::read_integer(src)? as u32;
            let max_user_ids = ber::read_integer(src)? as u32;
            let max_token_ids = ber::read_integer(src)? as u32;
            let num_priorities = ber::read_integer(src)? as u32;
            let min_throughput = ber::read_integer(src)? as u32;
            let max_height = ber::read_integer(src)? as u32;
            let max_mcs_pdu_size = ber::read_integer(src)? as u32;
            let protocol_version = ber::read_integer(src)? as u32;

            Ok(Self {
                max_channel_ids,
                max_user_ids,
                max_token_ids,
                num_priorities,
                min_throughput,
                max_height,
                max_mcs_pdu_size,
                protocol_version,
            })
        }
    }

    #[derive(Debug, Error)]
    pub enum McsError {
        #[error("IO error")]
        IOError(#[from] io::Error),
        #[error("RDP error")]
        RdpError(#[from] crate::RdpError),
        #[error("GCC block error")]
        GccError(#[from] GccError),
        #[error("invalid disconnect provider ultimatum")]
        InvalidDisconnectProviderUltimatum,
        #[error("invalid domain MCS PDU")]
        InvalidDomainMcsPdu,
        #[error("invalid MCS Connection Sequence PDU")]
        InvalidPdu(String),
        #[error("invalid invalid MCS channel id")]
        UnexpectedChannelId(String),
        #[error("PDU error: {0}")]
        Pdu(PduError),
    }

    impl From<PduError> for McsError {
        fn from(e: PduError) -> Self {
            Self::Pdu(e)
        }
    }

    impl From<McsError> for io::Error {
        fn from(e: McsError) -> io::Error {
            io::Error::new(io::ErrorKind::Other, format!("MCS Connection Sequence error: {e}"))
        }
    }
}
