//! USB device redirection ([MS-RDPEUSB]) wiring.
//!
//! The nusb backend performs USB I/O on its own threads and reports through [`UrbdrcEvent`]s. They
//! travel on the unbounded, ordered event queue, because dropping a completion would leave a server
//! driver waiting forever, and are applied here to the DRDYNVC client owned by the session loop.
//!
//! [MS-RDPEUSB]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

use std::sync::Arc;

use ironrdp_dvc::{DrdynvcClient, DvcMessageBatch, encode_dvc_messages};
use ironrdp_rdpeusb::client::{UrbdrcControlClient, UrbdrcDeviceClient, UrbdrcListener};
use ironrdp_rdpeusb::io::CompletionData;
use ironrdp_rdpeusb_native::{UrbdrcEvent, UsbRedirectionConfig};
use ironrdp_session::{ActiveStage, SessionError, SessionErrorExt as _, SessionResult};
use ironrdp_svc::ChannelFlags;
use tracing::{debug, trace, warn};

use crate::rdp::RdpInputSender;

/// What the session loop must send after applying an [`UrbdrcEvent`].
pub(crate) enum UrbdrcOutput {
    Dvc(DvcMessageBatch),
    Frame(Vec<u8>),
    Nothing,
}

/// Build the URBDRC listener, forwarding backend events to the session loop through `sender`.
pub(crate) fn urbdrc_listener(config: &UsbRedirectionConfig, sender: RdpInputSender) -> UrbdrcListener {
    ironrdp_rdpeusb_native::urbdrc_listener(
        config.clone(),
        Arc::new(move |event| {
            if sender.send_urbdrc(event).is_err() {
                trace!("Dropping a USB redirection event because the session ended");
            }
        }),
    )
}

/// Apply a backend event to the URBDRC channels.
pub(crate) fn process_urbdrc_event(active_stage: &mut ActiveStage, event: UrbdrcEvent) -> SessionResult<UrbdrcOutput> {
    let Some(drdynvc) = active_stage.get_svc_processor_mut::<DrdynvcClient>() else {
        return Ok(UrbdrcOutput::Nothing);
    };

    let (channel_id, message) = match event {
        UrbdrcEvent::Completion {
            channel_id,
            request_id,
            completion,
        } => {
            let Some(mut channel) = drdynvc.get_dvc_by_channel_id_mut::<UrbdrcDeviceClient>(channel_id) else {
                trace!(channel_id, request_id, "Dropping a USB completion for a closed device channel");
                return Ok(UrbdrcOutput::Nothing);
            };
            let device = channel.processor_mut();
            let message = match completion {
                CompletionData::IoControl(result) => device.io_ctl_completion(request_id, result),
                CompletionData::InternalIoControl(result) => device.internal_io_ctl_completion(request_id, result),
                CompletionData::TransferIn(result) => device.transfer_in_completion(request_id, result),
                CompletionData::TransferOut(result) => device.transfer_out_completion(request_id, result),
            };
            match message {
                Ok(message) => (channel_id, message),
                Err(error) => {
                    // The server cancelled the request, or retracted the device, in the meantime.
                    debug!(channel_id, request_id, %error, "Dropping a USB completion that is no longer pending");
                    return Ok(UrbdrcOutput::Nothing);
                }
            }
        }
        UrbdrcEvent::AddVirtualChannel { control_channel_id } => {
            let Some(channel) = drdynvc.get_dvc_by_channel_id::<UrbdrcControlClient>(control_channel_id) else {
                warn!(control_channel_id, "Cannot announce a USB device: the URBDRC control channel is closed");
                return Ok(UrbdrcOutput::Nothing);
            };
            match channel.processor().add_virtual_channel() {
                Ok(message) => (control_channel_id, message),
                Err(error) => {
                    warn!(%error, "Cannot announce a USB device");
                    return Ok(UrbdrcOutput::Nothing);
                }
            }
        }
        UrbdrcEvent::CloseChannel { channel_id } => {
            let Some(message) = drdynvc.close_channel(channel_id) else {
                return Ok(UrbdrcOutput::Nothing);
            };
            return Ok(UrbdrcOutput::Frame(active_stage.encode_dvc_messages(vec![message])?));
        }
    };

    let messages =
        encode_dvc_messages(channel_id, vec![message], ChannelFlags::empty()).map_err(SessionError::encode)?;
    Ok(UrbdrcOutput::Dvc(
        DvcMessageBatch::try_new(channel_id, messages).map_err(SessionError::pdu)?,
    ))
}
