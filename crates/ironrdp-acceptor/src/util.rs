use std::borrow::Cow;

use ironrdp_connector::{ConnectorError, ConnectorErrorExt, ConnectorResult};
use ironrdp_core::WriteBuf;
use ironrdp_pdu::{encode_vec, rdp, x224::X224, Encode};

pub(crate) fn encode_send_data_indication<T>(
    initiator_id: u16,
    channel_id: u16,
    user_msg: &T,
    buf: &mut WriteBuf,
) -> ConnectorResult<usize>
where
    T: Encode,
{
    let user_data = encode_vec(user_msg).map_err(ConnectorError::encode)?;

    let pdu = ironrdp_pdu::mcs::SendDataIndication {
        initiator_id,
        channel_id,
        user_data: Cow::Owned(user_data),
    };

    let written = ironrdp_pdu::encode_buf(&X224(pdu), buf).map_err(ConnectorError::encode)?;

    Ok(written)
}

pub(crate) fn wrap_share_data(pdu: rdp::headers::ShareDataPdu, io_channel_id: u16) -> rdp::headers::ShareControlHeader {
    rdp::headers::ShareControlHeader {
        share_id: 0,
        pdu_source: io_channel_id,
        share_control_pdu: rdp::headers::ShareControlPdu::Data(rdp::headers::ShareDataHeader {
            share_data_pdu: pdu,
            stream_priority: rdp::headers::StreamPriority::Undefined,
            compression_flags: rdp::headers::CompressionFlags::empty(),
            compression_type: rdp::client_info::CompressionType::K8,
        }),
    }
}
