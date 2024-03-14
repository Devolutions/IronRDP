mod display;
mod gfx;

use std::collections::HashMap;

use ironrdp_connector::connection_activation::ConnectionActivationSequence;
use ironrdp_connector::legacy::SendDataIndicationCtx;
use ironrdp_pdu::mcs::{DisconnectProviderUltimatum, DisconnectReason, McsMessage};
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_pdu::rdp::vc::dvc;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_svc::{client_encode_svc_messages, StaticChannelSet, SvcMessage, SvcProcessor, SvcProcessorMessages};

use crate::{SessionError, SessionErrorExt as _, SessionResult};

#[rustfmt::skip]
pub use self::gfx::GfxHandler;

pub const RDP8_GRAPHICS_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::Graphics";
pub const RDP8_DISPLAY_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

/// X224 Processor output
#[derive(Debug, Clone)]
pub enum ProcessorOutput {
    /// A buffer with encoded data to send to the server.
    ResponseFrame(Vec<u8>),
    /// A graceful disconnect notification. Client should close the connection upon receiving this.
    Disconnect(DisconnectReason),
    /// Received a [`ironrdp_pdu::rdp::headers::ServerDeactivateAll`] PDU. Client should execute the
    /// [Deactivation-Reactivation Sequence].
    ///
    /// [Deactivation-Reactivation Sequence]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
    DeactivateAll(ConnectionActivationSequence),
}

pub struct Processor {
    channel_map: HashMap<String, u32>,
    static_channels: StaticChannelSet,
    user_channel_id: u16,
    io_channel_id: u16,
    drdynvc_channel_id: Option<u16>,
    connection_activation: ConnectionActivationSequence,
}

impl Processor {
    pub fn new(
        static_channels: StaticChannelSet,
        user_channel_id: u16,
        io_channel_id: u16,
        connection_activation: ConnectionActivationSequence,
    ) -> Self {
        let drdynvc_channel_id = static_channels.iter().find_map(|(type_id, channel)| {
            if channel.is_drdynvc() {
                static_channels.get_channel_id_by_type_id(type_id)
            } else {
                None
            }
        });

        Self {
            static_channels,
            channel_map: HashMap::new(),
            user_channel_id,
            io_channel_id,
            drdynvc_channel_id,
            connection_activation,
        }
    }

    pub fn get_svc_processor<T: SvcProcessor + 'static>(&mut self) -> Option<&T> {
        self.static_channels
            .get_by_type::<T>()
            .and_then(|svc| svc.channel_processor_downcast_ref())
    }

    pub fn get_svc_processor_mut<T: SvcProcessor + 'static>(&mut self) -> Option<&mut T> {
        self.static_channels
            .get_by_type_mut::<T>()
            .and_then(|svc| svc.channel_processor_downcast_mut())
    }

    /// Completes user's SVC request with data, required to sent it over the network and returns
    /// a buffer with encoded data.
    pub fn process_svc_processor_messages<C: SvcProcessor + 'static>(
        &self,
        messages: SvcProcessorMessages<C>,
    ) -> SessionResult<Vec<u8>> {
        let channel_id = self
            .static_channels
            .get_channel_id_by_type::<C>()
            .ok_or_else(|| reason_err!("SVC", "channel not found"))?;

        process_svc_messages(messages.into(), channel_id, self.user_channel_id)
    }

    /// Processes a received PDU. Returns a vector of [`ProcessorOutput`] that must be processed
    /// in the returned order.
    pub fn process(&mut self, frame: &[u8]) -> SessionResult<Vec<ProcessorOutput>> {
        let data_ctx: SendDataIndicationCtx<'_> =
            ironrdp_connector::legacy::decode_send_data_indication(frame).map_err(crate::legacy::map_error)?;
        let channel_id = data_ctx.channel_id;

        if channel_id == self.io_channel_id {
            self.process_io_channel(data_ctx)
        } else if let Some(svc) = self.static_channels.get_by_channel_id_mut(channel_id) {
            let response_pdus = svc.process(data_ctx.user_data).map_err(crate::SessionError::pdu)?;
            process_svc_messages(response_pdus, channel_id, data_ctx.initiator_id)
                .map(|data| vec![ProcessorOutput::ResponseFrame(data)])
        } else {
            Err(reason_err!("X224", "unexpected channel received: ID {channel_id}"))
        }
    }

    fn process_io_channel(&self, data_ctx: SendDataIndicationCtx<'_>) -> SessionResult<Vec<ProcessorOutput>> {
        debug_assert_eq!(data_ctx.channel_id, self.io_channel_id);

        let io_channel = ironrdp_connector::legacy::decode_io_channel(data_ctx).map_err(crate::legacy::map_error)?;

        match io_channel {
            ironrdp_connector::legacy::IoChannelPdu::Data(ctx) => {
                match ctx.pdu {
                    ShareDataPdu::SaveSessionInfo(session_info) => {
                        debug!("Got Session Save Info PDU: {session_info:?}");
                        Ok(Vec::new())
                    }
                    ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
                        ProtocolIndependentCode::None,
                    ))) => {
                        debug!("Received None server error");
                        Ok(Vec::new())
                    }
                    ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(e)) => {
                        // This is a part of server-side graceful disconnect procedure defined
                        // in [MS-RDPBCGR].
                        //
                        // [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/149070b0-ecec-4c20-af03-934bbc48adb8
                        let graceful_disconnect = error_info_to_graceful_disconnect_reason(&e);

                        if let Some(reason) = graceful_disconnect {
                            debug!("Received server-side graceful disconnect request: {reason}");

                            Ok(vec![ProcessorOutput::Disconnect(reason)])
                        } else {
                            Err(reason_err!("ServerSetErrorInfo", "{}", e.description()))
                        }
                    }
                    ShareDataPdu::ShutdownDenied => {
                        debug!("ShutdownDenied received, session will be closed");

                        // As defined in [MS-RDPBCGR], when `ShareDataPdu::ShutdownDenied` is received, we
                        // need to send a disconnect ultimatum to the server if we want to proceed with the
                        // session shutdown.
                        //
                        // [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/27915739-8f77-487e-9927-55008af7fd68
                        let ultimatum = McsMessage::DisconnectProviderUltimatum(
                            DisconnectProviderUltimatum::from_reason(DisconnectReason::UserRequested),
                        );

                        let encoded_pdu = ironrdp_pdu::encode_vec(&ultimatum).map_err(SessionError::pdu);

                        Ok(vec![
                            ProcessorOutput::ResponseFrame(encoded_pdu?),
                            ProcessorOutput::Disconnect(DisconnectReason::UserRequested),
                        ])
                    }
                    _ => Err(reason_err!(
                        "IO channel",
                        "unexpected PDU: expected Session Save Info PDU, got: {:?}",
                        ctx.pdu.as_short_name()
                    )),
                }
            }
            ironrdp_connector::legacy::IoChannelPdu::DeactivateAll(_) => Ok(vec![ProcessorOutput::DeactivateAll(
                self.connection_activation.reset_clone(),
            )]),
        }
    }

    /// Sends a PDU on the dynamic channel.
    // pub fn encode_dynamic(&self, output: &mut WriteBuf, channel_name: &str, dvc_data: &[u8]) -> SessionResult<()> {
    //     let drdynvc_channel_id = self
    //         .drdynvc_channel_id
    //         .ok_or_else(|| general_err!("dynamic virtual channel not connected"))?;

    //     let dvc_channel_id = self
    //         .channel_map
    //         .get(channel_name)
    //         .ok_or_else(|| reason_err!("DVC", "access to non existing channel name: {}", channel_name))?;

    //     let dvc_channel = self
    //         .dynamic_channels
    //         .get(dvc_channel_id)
    //         .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", dvc_channel_id))?;

    //     let dvc_client_data = dvc::ClientPdu::Common(dvc::CommonPdu::Data(dvc::DataPdu {
    //         channel_id_type: dvc_channel.channel_id_type,
    //         channel_id: dvc_channel.channel_id,
    //         data_size: dvc_data.len(),
    //     }));

    //     crate::legacy::encode_dvc_message(
    //         self.user_channel_id,
    //         drdynvc_channel_id,
    //         dvc_client_data,
    //         dvc_data,
    //         output,
    //     )?;

    //     Ok(())
    // }

    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn encode_static(&self, output: &mut WriteBuf, pdu: ShareDataPdu) -> SessionResult<usize> {
        let written =
            ironrdp_connector::legacy::encode_share_data(self.user_channel_id, self.io_channel_id, 0, pdu, output)
                .map_err(crate::legacy::map_error)?;
        Ok(written)
    }
}

/// Processes a vector of [`SvcMessage`] in preparation for sending them to the server on the `channel_id` channel.
///
/// This includes chunkifying the messages, adding MCS, x224, and tpkt headers, and encoding them into a buffer.
/// The messages returned here are ready to be sent to the server.
///
/// The caller is responsible for ensuring that the `channel_id` corresponds to the correct channel.
fn process_svc_messages(messages: Vec<SvcMessage>, channel_id: u16, initiator_id: u16) -> SessionResult<Vec<u8>> {
    client_encode_svc_messages(messages, channel_id, initiator_id).map_err(crate::SessionError::pdu)
}

/// Converts an [`ErrorInfo`] into a [`DisconnectReason`].
///
/// Returns `None` if the error code is not a graceful disconnect code.
pub fn error_info_to_graceful_disconnect_reason(error_info: &ErrorInfo) -> Option<DisconnectReason> {
    let code = if let ErrorInfo::ProtocolIndependentCode(code) = error_info {
        code
    } else {
        return None;
    };

    match code {
        ProtocolIndependentCode::RpcInitiatedDisconnect
        | ProtocolIndependentCode::RpcInitiatedLogoff
        | ProtocolIndependentCode::DisconnectedByOtherconnection => Some(DisconnectReason::ProviderInitiated),
        ProtocolIndependentCode::RpcInitiatedDisconnectByuser | ProtocolIndependentCode::LogoffByUser => {
            Some(DisconnectReason::UserRequested)
        }
        _ => None,
    }
}
