use std::borrow::Cow;

use ironrdp_core::{decode, encode_vec, Decode, Encode, WriteBuf};
use ironrdp_pdu::rdp;
use ironrdp_pdu::rdp::headers::{BasicSecurityHeaderFlags, ServerDeactivateAll, BASIC_SECURITY_HEADER_SIZE};
use ironrdp_pdu::rdp::multitransport::MultitransportRequestPdu;
use ironrdp_pdu::x224::X224;

use crate::{general_err, reason_err, ConnectorError, ConnectorErrorExt as _, ConnectorResult};

pub fn encode_send_data_request<T>(
    initiator_id: u16,
    channel_id: u16,
    user_msg: &T,
    buf: &mut WriteBuf,
) -> ConnectorResult<usize>
where
    T: Encode,
{
    let user_data = encode_vec(user_msg).map_err(ConnectorError::encode)?;

    let pdu = ironrdp_pdu::mcs::SendDataRequest {
        initiator_id,
        channel_id,
        user_data: Cow::Owned(user_data),
    };

    let written = ironrdp_core::encode_buf(&X224(pdu), buf).map_err(ConnectorError::encode)?;

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
        T: Decode<'de>,
        'a: 'de,
    {
        let msg = decode::<T>(self.user_data).map_err(ConnectorError::decode)?;
        Ok(msg)
    }
}

pub fn decode_send_data_indication(src: &[u8]) -> ConnectorResult<SendDataIndicationCtx<'_>> {
    use ironrdp_pdu::mcs::McsMessage;

    let mcs_msg = decode::<X224<McsMessage<'_>>>(src).map_err(ConnectorError::decode)?;

    match mcs_msg.0 {
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
        _ => Err(reason_err!(
            "decode_send_data_indication",
            "unexpected MCS message: {}",
            ironrdp_core::name(&mcs_msg)
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
    /// Server Initiate Multitransport Request PDU.
    ///
    /// Received when the server wants the client to establish a sideband UDP transport.
    MultitransportRequest(MultitransportRequestPdu),
}

pub fn decode_io_channel(ctx: SendDataIndicationCtx<'_>) -> ConnectorResult<IoChannelPdu> {
    fn try_decode_multitransport(data: &[u8]) -> Option<MultitransportRequestPdu> {
        if data.len() < BASIC_SECURITY_HEADER_SIZE {
            return None;
        }

        let flags_raw = u16::from_le_bytes([data[0], data[1]]);
        let flags_hi = u16::from_le_bytes([data[2], data[3]]);

        // ShareControlHeader always has pduType | PROTOCOL_VERSION (>= 0x11) at bytes [2..4].
        // BasicSecurityHeader has flagsHi == 0 for non-encrypted PDUs.
        if flags_hi != 0 {
            return None;
        }

        let flags = BasicSecurityHeaderFlags::from_bits(flags_raw)?;

        if !flags.contains(BasicSecurityHeaderFlags::TRANSPORT_REQ) {
            return None;
        }

        decode::<MultitransportRequestPdu>(data).ok()
    }

    // Multitransport PDUs use BasicSecurityHeader (flags:u16, flagsHi:u16) instead
    // of the ShareControlHeader (totalLength:u16, pduType_with_version:u16, ...) used
    // by all other IO channel PDUs. We discriminate by checking bytes [2..4] == 0:
    // ShareControl always has pduType | PROTOCOL_VERSION (>= 0x11) at that offset.
    if let Some(pdu) = try_decode_multitransport(ctx.user_data) {
        return Ok(IoChannelPdu::MultitransportRequest(pdu));
    }

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
