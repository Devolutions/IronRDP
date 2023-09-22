#![cfg_attr(not(feature = "std"), no_std)]
#![allow(unused)] // FIXME: remove this annotation

// TODO: this crate is WIP

#[macro_use]
extern crate tracing;

extern crate alloc;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::rdp::vc;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{assert_obj_safe, dvc, PduResult};
use ironrdp_svc::{impl_as_any, ChunkProcessor, CompressionCondition, StaticVirtualChannel, SvcMessage};
use pdu::cursor::WriteCursor;
use pdu::PduEncode;

/// A type that is a Dynamic Virtual Channel (DVC)
///
/// Dynamic virtual channels may be created at any point during the RDP session.
/// The Dynamic Virtual Channel APIs exist to address limitations of Static Virtual Channels:
///   - Limited number of channels
///   - Packet reconstruction
pub trait DynamicVirtualChannel: Send + Sync {
    fn channel_name(&self) -> &str;

    fn process(&mut self, channel_id: u32, payload: &[u8], output: &mut WriteBuf) -> PduResult<()>;
}

assert_obj_safe!(DynamicVirtualChannel);

/// DRDYNVC Static Virtual Channel (the Remote Desktop Protocol: Dynamic Virtual Channel Extension)
///
/// It adds support for dynamic virtual channels (DVC).
pub struct Drdynvc {
    dynamic_channels: BTreeMap<String, Box<dyn DynamicVirtualChannel>>,
}

impl fmt::Debug for Drdynvc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Drdynvc([")?;

        let mut is_first = true;

        for channel in self.dynamic_channels.values() {
            if is_first {
                is_first = false;
            } else {
                write!(f, ", ")?;
            }

            write!(f, "{}", channel.channel_name())?;
        }

        write!(f, "])")
    }
}

impl Drdynvc {
    pub const NAME: ChannelName = ChannelName::from_static(b"drdynvc\0");

    pub fn new() -> Self {
        Self {
            dynamic_channels: BTreeMap::new(),
        }
    }

    // FIXME: it’s likely we want to enable adding dynamic channels at any point during the session (message passing? other approach?)

    pub fn with_dynamic_channel<T>(mut self, channel: T) -> Self
    where
        T: DynamicVirtualChannel + 'static,
    {
        let channel_name = channel.channel_name().to_owned();
        self.dynamic_channels.insert(channel_name, Box::new(channel));
        self
    }
}

impl_as_any!(Drdynvc);

impl Default for Drdynvc {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticVirtualChannel for Drdynvc {
    fn channel_name(&self) -> ChannelName {
        Drdynvc::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let dvc_ctx = decode_dvc_message(payload)?;

        match dvc_ctx.dvc_pdu {
            dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
                debug!("Got DVC Capabilities Request PDU: {caps_request:?}");
                let caps_response = dvc::ClientPdu::CapabilitiesResponse(dvc::CapabilitiesResponsePdu {
                    version: dvc::CapsVersion::V1,
                });

                debug!("Send DVC Capabilities Response PDU: {caps_response:?}");
                // crate::legacy::encode_dvc_message(initiator_id, channel_id, caps_response, &[], output)?;
            }
            dvc::ServerPdu::CreateRequest(create_request) => {
                debug!("Got DVC Create Request PDU: {create_request:?}");

                // let creation_status = if let Some(dynamic_channel) = create_dvc(
                //     create_request.channel_name.as_str(),
                //     create_request.channel_id,
                //     create_request.channel_id_type,
                //     &mut self.graphics_handler,
                // ) {
                //     self.dynamic_channels.insert(create_request.channel_id, dynamic_channel);
                //     self.channel_map
                //         .insert(create_request.channel_name.clone(), create_request.channel_id);

                //     dvc::DVC_CREATION_STATUS_OK
                // } else {
                //     dvc::DVC_CREATION_STATUS_NO_LISTENER
                // };

                // let create_response = dvc::ClientPdu::CreateResponse(dvc::CreateResponsePdu {
                //     channel_id_type: create_request.channel_id_type,
                //     channel_id: create_request.channel_id,
                //     creation_status,
                // });

                // debug!("Send DVC Create Response PDU: {create_response:?}");
                // crate::legacy::encode_dvc_message(
                //     data_ctx.initiator_id,
                //     data_ctx.channel_id,
                //     create_response,
                //     &[],
                //     &mut buf,
                // )?;

                // negotiate_dvc(
                //     &create_request,
                //     data_ctx.initiator_id,
                //     data_ctx.channel_id,
                //     &mut buf,
                //     &self.graphics_config,
                // )?;
            }
            dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {close_request:?}");

                let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                    channel_id_type: close_request.channel_id_type,
                    channel_id: close_request.channel_id,
                });

                // debug!("Send DVC Close Response PDU: {close_response:?}");
                // crate::legacy::encode_dvc_message(
                //     data_ctx.initiator_id,
                //     data_ctx.channel_id,
                //     close_response,
                //     &[],
                //     &mut buf,
                // )?;

                // self.dynamic_channels.remove(&close_request.channel_id);
            }
            dvc::ServerPdu::DataFirst(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let dvc_data = dvc_ctx.dvc_data;

                // // FIXME(perf): copy with data_buf.to_vec()
                // if let Some(dvc_data) = self
                //     .dynamic_channels
                //     .get_mut(&data.channel_id)
                //     .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", data.channel_id))?
                //     .process_data_first_pdu(data.total_data_size as usize, dvc_data.to_vec())?
                // {
                //     let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                //         channel_id_type,
                //         channel_id,
                //         data_size: dvc_data.len(),
                //     });

                //     crate::legacy::encode_dvc_message(
                //         data_ctx.initiator_id,
                //         data_ctx.channel_id,
                //         client_data,
                //         &dvc_data,
                //         &mut buf,
                //     )?;
                // }
            }
            dvc::ServerPdu::Data(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let dvc_data = dvc_ctx.dvc_data;

                // // FIXME(perf): copy with data_buf.to_vec()
                // if let Some(dvc_data) = self
                //     .dynamic_channels
                //     .get_mut(&data.channel_id)
                //     .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", data.channel_id))?
                //     .process_data_pdu(dvc_data.to_vec())?
                // {
                //     let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                //         channel_id_type,
                //         channel_id,
                //         data_size: dvc_data.len(),
                //     });

                //     crate::legacy::encode_dvc_message(
                //         data_ctx.initiator_id,
                //         data_ctx.channel_id,
                //         client_data,
                //         &dvc_data,
                //         &mut buf,
                //     )?;
                // }
            }
        }

        Err(ironrdp_pdu::other_err!(
            "DRDYNVC",
            "ironrdp-dvc::Drdynvc implementation is not yet ready"
        ))
    }

    fn is_drdynvc(&self) -> bool {
        true
    }
}

struct DynamicChannelCtx<'a> {
    pub dvc_pdu: vc::dvc::ServerPdu,
    pub dvc_data: &'a [u8],
}

fn decode_dvc_message(user_data: &[u8]) -> PduResult<DynamicChannelCtx<'_>> {
    use ironrdp_pdu::{custom_err, PduParsing as _};

    let mut user_data = user_data;
    let user_data_len = user_data.len();

    // [ vc::ChannelPduHeader | …
    let channel_header = vc::ChannelPduHeader::from_buffer(&mut user_data).map_err(|e| custom_err!("DVC header", e))?;
    debug_assert_eq!(user_data_len, channel_header.length as usize);

    // … | dvc::ServerPdu | …
    let dvc_pdu =
        vc::dvc::ServerPdu::from_buffer(&mut user_data, user_data_len).map_err(|e| custom_err!("DVC server PDU", e))?;

    // … | DvcData ]
    let dvc_data = user_data;

    Ok(DynamicChannelCtx { dvc_pdu, dvc_data })
}
