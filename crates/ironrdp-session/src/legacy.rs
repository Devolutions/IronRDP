use ironrdp_connector::encode_send_data_request;
use ironrdp_connector::legacy::SendDataIndicationCtx;
use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::rdp::vc;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{decode, PduEncode, PduResult};

use crate::{SessionError, SessionErrorExt, SessionResult};

pub fn encode_dvc_message(
    initiator_id: u16,
    drdynvc_id: u16,
    dvc_pdu: vc::dvc::ClientPdu,
    dvc_data: &[u8],
    buf: &mut WriteBuf,
) -> SessionResult<()> {
    let dvc_length = dvc_pdu.size() + dvc_data.len();

    let channel_header = vc::ChannelPduHeader {
        length: u32::try_from(dvc_length).expect("dvc message size"),
        flags: vc::ChannelControlFlags::FLAG_FIRST | vc::ChannelControlFlags::FLAG_LAST,
    };

    let dvc_message = DvcMessage {
        channel_header,
        dvc_pdu,
        dvc_data,
    };

    let previous_length = buf.filled_len();
    // [ TPKT | TPDU | SendDataRequest | vc::ChannelPduHeader | vc::dvc::ClientPdu | DvcData ]
    let written = encode_send_data_request(initiator_id, drdynvc_id, &dvc_message, buf).map_err(map_error)?;
    debug_assert_eq!(written, buf.filled_len() - previous_length);

    Ok(())
}

struct DvcMessage<'a> {
    channel_header: vc::ChannelPduHeader,
    dvc_pdu: vc::dvc::ClientPdu,
    dvc_data: &'a [u8],
}

impl DvcMessage<'_> {
    const NAME: &'static str = "DvcMessage";
}

impl PduEncode for DvcMessage<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ironrdp_pdu::ensure_size!(in: dst, size: self.size());

        self.channel_header.encode(dst)?;
        self.dvc_pdu.encode(dst)?;
        dst.write_slice(self.dvc_data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.channel_header.size() + self.dvc_pdu.size() + self.dvc_data.len()
    }
}

pub struct DynamicChannelCtx<'a> {
    pub dvc_pdu: vc::dvc::ServerPdu,
    pub dvc_data: &'a [u8],
}

pub fn decode_dvc_message(ctx: SendDataIndicationCtx<'_>) -> SessionResult<DynamicChannelCtx<'_>> {
    let user_data = ctx.user_data;

    // [ vc::ChannelPduHeader | …
    let channel_header: vc::ChannelPduHeader = decode(user_data).map_err(SessionError::pdu)?;
    let dvc_data_len = user_data.len();
    debug_assert_eq!(dvc_data_len, channel_header.length as usize);

    // … | dvc::ServerPdu | …
    let mut cur = ReadCursor::new(user_data);
    let dvc_pdu = vc::dvc::ServerPdu::decode(&mut cur, dvc_data_len).map_err(SessionError::pdu)?;

    // … | DvcData ]
    let dvc_data = cur.remaining();

    Ok(DynamicChannelCtx { dvc_pdu, dvc_data })
}

// FIXME: code should be fixed so that we never need this conversion
// For that, some code from this ironrdp_session::legacy and ironrdp_connector::legacy modules should be moved to ironrdp_pdu itself
impl From<ironrdp_connector::ConnectorErrorKind> for crate::SessionErrorKind {
    fn from(value: ironrdp_connector::ConnectorErrorKind) -> Self {
        match value {
            ironrdp_connector::ConnectorErrorKind::Pdu(e) => crate::SessionErrorKind::Pdu(e),
            ironrdp_connector::ConnectorErrorKind::Credssp(_) => panic!("unexpected"),
            ironrdp_connector::ConnectorErrorKind::AccessDenied => panic!("unexpected"),
            ironrdp_connector::ConnectorErrorKind::General => crate::SessionErrorKind::General,
            ironrdp_connector::ConnectorErrorKind::Custom => crate::SessionErrorKind::Custom,
            _ => crate::SessionErrorKind::General,
        }
    }
}

pub(crate) fn map_error(error: ironrdp_connector::ConnectorError) -> SessionError {
    error.into_other_kind()
}

impl ironrdp_error::legacy::CatchAllKind for crate::SessionErrorKind {
    const CATCH_ALL_VALUE: Self = crate::SessionErrorKind::General;
}
