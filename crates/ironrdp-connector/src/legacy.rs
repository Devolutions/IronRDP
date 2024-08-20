use std::borrow::Cow;

use ironrdp_core::WriteBuf;
use ironrdp_pdu::rdp::headers::ServerDeactivateAll;
use ironrdp_pdu::{decode, encode_vec, rdp, PduDecode, PduEncode};

use crate::{ConnectorError, ConnectorErrorExt as _, ConnectorResult};

pub fn encode_send_data_request<T>(
    initiator_id: u16,
    channel_id: u16,
    user_msg: &T,
    buf: &mut WriteBuf,
) -> ConnectorResult<usize>
where
    T: PduEncode,
{
    let user_data = encode_vec(user_msg).map_err(ConnectorError::pdu)?;

    let pdu = ironrdp_pdu::mcs::SendDataRequest {
        initiator_id,
        channel_id,
        user_data: Cow::Owned(user_data),
    };

    let written = ironrdp_pdu::encode_buf(&pdu, buf).map_err(ConnectorError::pdu)?;

    Ok(written)
}

#[derive(Debug, Clone, Copy)]
pub struct SendDataIndicationCtx<'a> {
    pub initiator_id: u16,
    pub channel_id: u16,
    pub user_data: &'a [u8],
}

impl<'a> SendDataIndicationCtx<'a> {
    pub fn decode_user_data<'de, T>(&self) -> ConnectorResult<T>
    where
        T: PduDecode<'de>,
        'a: 'de,
    {
        let msg = decode::<T>(self.user_data).map_err(ConnectorError::pdu)?;
        Ok(msg)
    }
}

pub fn decode_send_data_indication(src: &[u8]) -> ConnectorResult<SendDataIndicationCtx<'_>> {
    use ironrdp_pdu::mcs::McsMessage;

    let mcs_msg = decode::<McsMessage<'_>>(src).map_err(ConnectorError::pdu)?;

    match mcs_msg {
        McsMessage::SendDataIndication(msg) => {
            let Cow::Borrowed(user_data) = msg.user_data else {
                unreachable!()
            };

            Ok(SendDataIndicationCtx {
                initiator_id: msg.initiator_id,
                channel_id: msg.channel_id,
                user_data,
            })
        }
        McsMessage::DisconnectProviderUltimatum(msg) => Err(reason_err!(
            "decode_send_data_indication",
            "received disconnect provider ultimatum: {:?}",
            msg.reason
        )),
        unexpected => Err(reason_err!(
            "decode_send_data_indication",
            "unexpected MCS message: {}",
            ironrdp_pdu::name(&unexpected)
        )),
    }
}

pub fn encode_share_control(
    initiator_id: u16,
    channel_id: u16,
    share_id: u32,
    pdu: rdp::headers::ShareControlPdu,
    buf: &mut WriteBuf,
) -> ConnectorResult<usize> {
    let pdu_source = initiator_id;

    let share_control_header = rdp::headers::ShareControlHeader {
        share_control_pdu: pdu,
        pdu_source,
        share_id,
    };

    encode_send_data_request(initiator_id, channel_id, &share_control_header, buf)
}

#[derive(Debug, Clone)]
pub struct ShareControlCtx {
    pub initiator_id: u16,
    pub channel_id: u16,
    pub share_id: u32,
    pub pdu_source: u16,
    pub pdu: rdp::headers::ShareControlPdu,
}

pub fn decode_share_control(ctx: SendDataIndicationCtx<'_>) -> ConnectorResult<ShareControlCtx> {
    let user_msg = ctx.decode_user_data::<rdp::headers::ShareControlHeader>()?;

    Ok(ShareControlCtx {
        initiator_id: ctx.initiator_id,
        channel_id: ctx.channel_id,
        share_id: user_msg.share_id,
        pdu_source: user_msg.pdu_source,
        pdu: user_msg.share_control_pdu,
    })
}

pub fn encode_share_data(
    initiator_id: u16,
    channel_id: u16,
    share_id: u32,
    pdu: rdp::headers::ShareDataPdu,
    buf: &mut WriteBuf,
) -> ConnectorResult<usize> {
    let share_data_header = rdp::headers::ShareDataHeader {
        share_data_pdu: pdu,
        stream_priority: rdp::headers::StreamPriority::Medium,
        compression_flags: rdp::headers::CompressionFlags::empty(),
        compression_type: rdp::client_info::CompressionType::K8, // ignored if CompressionFlags::empty()
    };

    let share_control_pdu = rdp::headers::ShareControlPdu::Data(share_data_header);

    encode_share_control(initiator_id, channel_id, share_id, share_control_pdu, buf)
}

#[derive(Debug, Clone)]
pub struct ShareDataCtx {
    pub initiator_id: u16,
    pub channel_id: u16,
    pub share_id: u32,
    pub pdu_source: u16,
    pub pdu: rdp::headers::ShareDataPdu,
}

pub fn decode_share_data(ctx: SendDataIndicationCtx<'_>) -> ConnectorResult<ShareDataCtx> {
    let ctx = decode_share_control(ctx)?;

    let rdp::headers::ShareControlPdu::Data(share_data_header) = ctx.pdu else {
        return Err(general_err!(
            "received unexpected Share Control Pdu (expected Share Data Header)"
        ));
    };

    Ok(ShareDataCtx {
        initiator_id: ctx.initiator_id,
        channel_id: ctx.channel_id,
        share_id: ctx.share_id,
        pdu_source: ctx.pdu_source,
        pdu: share_data_header.share_data_pdu,
    })
}

pub enum IoChannelPdu {
    Data(ShareDataCtx),
    DeactivateAll(ServerDeactivateAll),
}

pub fn decode_io_channel(ctx: SendDataIndicationCtx<'_>) -> ConnectorResult<IoChannelPdu> {
    let ctx = decode_share_control(ctx)?;

    match ctx.pdu {
        rdp::headers::ShareControlPdu::ServerDeactivateAll(deactivate_all) => {
            Ok(IoChannelPdu::DeactivateAll(deactivate_all))
        }
        rdp::headers::ShareControlPdu::Data(share_data_header) => {
            let share_data_ctx = ShareDataCtx {
                initiator_id: ctx.initiator_id,
                channel_id: ctx.channel_id,
                share_id: ctx.share_id,
                pdu_source: ctx.pdu_source,
                pdu: share_data_header.share_data_pdu,
            };

            Ok(IoChannelPdu::Data(share_data_ctx))
        }
        _ => Err(general_err!(
            "received unexpected Share Control Pdu (expected Share Data Header or Server Deactivate All)"
        )),
    }
}

impl ironrdp_error::legacy::CatchAllKind for crate::ConnectorErrorKind {
    const CATCH_ALL_VALUE: Self = crate::ConnectorErrorKind::General;
}
