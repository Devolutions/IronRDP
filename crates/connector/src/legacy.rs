//! Legacy compat layer based on the old PduParsing trait

use std::borrow::Cow;

use ironrdp_pdu::{rdp, x224, PduParsing};

pub fn encode_x224_packet<T: PduParsing>(x224_msg: &T, buf: &mut Vec<u8>) -> crate::Result<usize>
where
    T: PduParsing,
    crate::Error: From<T::Error>,
{
    let x224_msg_len = x224_msg.buffer_length();
    let mut x224_msg_buf = Vec::with_capacity(x224_msg_len);

    x224_msg.to_buffer(&mut x224_msg_buf)?;

    let pdu = x224::X224Data {
        data: Cow::Owned(x224_msg_buf),
    };

    let written = ironrdp_pdu::encode_buf(&pdu, buf)?;

    Ok(written)
}

pub fn decode_x224_packet<T>(src: &[u8]) -> crate::Result<T>
where
    T: PduParsing,
    crate::Error: From<T::Error>,
{
    let x224_payload = ironrdp_pdu::decode::<x224::X224Data>(src)?;
    let x224_msg = T::from_buffer(x224_payload.data.as_ref())?;
    Ok(x224_msg)
}

pub fn encode_send_data_request<T>(
    initiator_id: u16,
    channel_id: u16,
    user_msg: &T,
    buf: &mut Vec<u8>,
) -> crate::Result<usize>
where
    T: PduParsing,
    crate::Error: From<T::Error>,
{
    let user_data_len = user_msg.buffer_length();
    let mut user_data = Vec::with_capacity(user_data_len);

    user_msg.to_buffer(&mut user_data)?;

    let pdu = ironrdp_pdu::mcs::SendDataRequest {
        initiator_id,
        channel_id,
        user_data: Cow::Owned(user_data),
    };

    let written = ironrdp_pdu::encode_buf(&pdu, buf)?;

    Ok(written)
}

#[derive(Debug, Clone, Copy)]
pub struct SendDataIndicationCtx<'a> {
    pub initiator_id: u16,
    pub channel_id: u16,
    pub user_data: &'a [u8],
}

impl SendDataIndicationCtx<'_> {
    pub fn decode_user_data<T>(&self) -> crate::Result<T>
    where
        T: PduParsing,
        crate::Error: From<T::Error>,
    {
        let msg = T::from_buffer(self.user_data)?;
        Ok(msg)
    }
}

pub fn decode_send_data_indication(src: &[u8]) -> crate::Result<SendDataIndicationCtx<'_>> {
    use ironrdp_pdu::mcs::McsMessage;

    let mcs_msg = ironrdp_pdu::decode::<McsMessage>(src)?;

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
        McsMessage::DisconnectProviderUltimatum(msg) => {
            Err(crate::Error::new("received disconnect provider ultimatum").with_reason(format!("{:?}", msg.reason)))
        }
        unexpected => Err(crate::Error::new("unexpected MCS message").with_reason(ironrdp_pdu::name(&unexpected))),
    }
}

pub fn encode_share_control(
    initiator_id: u16,
    channel_id: u16,
    share_id: u32,
    pdu: rdp::headers::ShareControlPdu,
    buf: &mut Vec<u8>,
) -> crate::Result<usize> {
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

pub fn decode_share_control(ctx: SendDataIndicationCtx<'_>) -> crate::Result<ShareControlCtx> {
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
    buf: &mut Vec<u8>,
) -> crate::Result<usize> {
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

pub fn decode_share_data(ctx: SendDataIndicationCtx<'_>) -> crate::Result<ShareDataCtx> {
    let ctx = decode_share_control(ctx)?;

    let rdp::headers::ShareControlPdu::Data(share_data_header) = ctx.pdu else {
        return Err(crate::Error::new("received unexpected Share Control Pdu (expected SHare Data Header)"));
    };

    Ok(ShareDataCtx {
        initiator_id: ctx.initiator_id,
        channel_id: ctx.channel_id,
        share_id: ctx.share_id,
        pdu_source: ctx.pdu_source,
        pdu: share_data_header.share_data_pdu,
    })
}

impl From<ironrdp_pdu::mcs::McsError> for crate::Error {
    fn from(e: ironrdp_pdu::mcs::McsError) -> Self {
        Self::new("MCS").with_reason(e.to_string())
    }
}

impl From<ironrdp_pdu::rdp::server_license::ServerLicenseError> for crate::Error {
    fn from(e: ironrdp_pdu::rdp::server_license::ServerLicenseError) -> Self {
        Self::new("server license").with_reason(e.to_string())
    }
}

impl From<ironrdp_pdu::rdp::RdpError> for crate::Error {
    fn from(e: ironrdp_pdu::rdp::RdpError) -> Self {
        Self::new("RDP").with_reason(e.to_string())
    }
}

impl From<ironrdp_pdu::rdp::vc::ChannelError> for crate::Error {
    fn from(e: ironrdp_pdu::rdp::vc::ChannelError) -> Self {
        Self::new("virtual channel").with_reason(e.to_string())
    }
}
