use ironrdp_connector::legacy::{encode_send_data_request, SendDataIndicationCtx};
use ironrdp_pdu::rdp::vc;
use ironrdp_pdu::PduParsing as _;

pub fn encode_dvc_message(
    initiator_id: u16,
    drdynvc_id: u16,
    dvc_pdu: vc::dvc::ClientPdu,
    dvc_data: &[u8],
    mut buf: &mut Vec<u8>,
) -> crate::Result<usize> {
    let dvc_length = dvc_pdu.buffer_length() + dvc_data.len();

    let channel_header = vc::ChannelPduHeader {
        length: u32::try_from(dvc_length).expect("dvc message size"),
        flags: vc::ChannelControlFlags::FLAG_FIRST | vc::ChannelControlFlags::FLAG_LAST,
    };

    // [ TPKT | TPDU | SendDataRequest | vc::ChannelPduHeader | …
    let written = encode_send_data_request(initiator_id, drdynvc_id, &channel_header, buf)?;
    buf.truncate(written);

    // … | dvc::ClientPdu | …
    dvc_pdu.to_buffer(&mut buf)?;

    // … | DvcData ]
    buf.extend_from_slice(dvc_data);

    debug_assert_eq!(buf.len(), written + dvc_length);

    Ok(written + dvc_length)
}

pub struct DynamicChannelCtx<'a> {
    pub dvc_pdu: vc::dvc::ServerPdu,
    pub dvc_data: &'a [u8],
}

pub fn decode_dvc_message(ctx: SendDataIndicationCtx<'_>) -> crate::Result<DynamicChannelCtx<'_>> {
    let mut user_data = ctx.user_data;
    let user_data_len = user_data.len();

    // [ vc::ChannelPduHeader | …
    let channel_header = vc::ChannelPduHeader::from_buffer(&mut user_data)?;
    debug_assert_eq!(user_data_len, channel_header.length as usize);

    // … | dvc::ServerPdu | …
    let dvc_pdu = vc::dvc::ServerPdu::from_buffer(&mut user_data, user_data_len)?;

    // … | DvcData ]
    let dvc_data = user_data;

    Ok(DynamicChannelCtx { dvc_pdu, dvc_data })
}

impl From<ironrdp_pdu::rdp::vc::ChannelError> for crate::Error {
    fn from(e: ironrdp_pdu::rdp::vc::ChannelError) -> Self {
        Self::new("virtual channel error").with_custom(e)
    }
}

// FIXME: code should be fixed so that we never need this conversion
// For that, some code from this ironrdp_session::legacy and ironrdp_connector::legacy modules should be moved to ironrdp_pdu itself
impl From<ironrdp_connector::Error> for crate::Error {
    fn from(value: ironrdp_connector::Error) -> Self {
        Self {
            context: value.context,
            kind: match value.kind {
                ironrdp_connector::ErrorKind::Pdu(e) => crate::ErrorKind::Pdu(e),
                ironrdp_connector::ErrorKind::Credssp(_) => panic!("unexpected"),
                ironrdp_connector::ErrorKind::AccessDenied => panic!("unexpected"),
                ironrdp_connector::ErrorKind::Custom(e) => crate::ErrorKind::Custom(e),
                ironrdp_connector::ErrorKind::General => crate::ErrorKind::General,
                _ => crate::ErrorKind::General,
            },
            reason: value.reason,
        }
    }
}

impl From<ironrdp_pdu::fast_path::FastPathError> for crate::Error {
    fn from(e: ironrdp_pdu::fast_path::FastPathError) -> Self {
        Self::new("Fast-Path").with_custom(e)
    }
}

impl From<ironrdp_pdu::codecs::rfx::RfxError> for crate::Error {
    fn from(e: ironrdp_pdu::codecs::rfx::RfxError) -> Self {
        Self::new("RFX").with_custom(e)
    }
}

impl From<ironrdp_pdu::dvc::display::DisplayPipelineError> for crate::Error {
    fn from(e: ironrdp_pdu::dvc::display::DisplayPipelineError) -> Self {
        Self::new("display pipeline").with_custom(e)
    }
}

impl From<ironrdp_graphics::zgfx::ZgfxError> for crate::Error {
    fn from(e: ironrdp_graphics::zgfx::ZgfxError) -> Self {
        Self::new("zgfx").with_reason(e.to_string())
    }
}

impl From<ironrdp_pdu::dvc::gfx::GraphicsPipelineError> for crate::Error {
    fn from(e: ironrdp_pdu::dvc::gfx::GraphicsPipelineError) -> Self {
        Self::new("graphics pipeline").with_reason(e.to_string())
    }
}
