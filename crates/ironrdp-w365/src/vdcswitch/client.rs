//! Client state machine for the virtual desktop control dynamic channel.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use super::CHANNEL_NAME;
use super::pdu::{LaunchUri, LocalAction, ProtocolVersion, VdcSwitchPdu, VersionRequestPdu};

/// Host behavior for the requests a Cloud PC sends over the channel.
///
/// The processor calls these methods from the session loop, so they must return promptly.
pub trait VdcSwitchHandler: Send {
    /// The Cloud PC asked the client to switch the user to the local desktop.
    ///
    /// Return `Some(hresult)` to answer with a switch-to-local response carrying that HRESULT.
    /// Return `None` to send no response, as Windows App does when it hands the switch to its container.
    fn switch_to_local_requested(&mut self) -> Option<u32>;

    /// The Cloud PC asked the client to perform a local action, such as opening the Bluetooth settings.
    ///
    /// The processor calls this only after the server selected version 2 or later.
    fn perform_local_action(&mut self, action: LocalAction);

    /// The Cloud PC answered a switch-to-cloud request with an HRESULT.
    fn switch_to_cloud_completed(&mut self, hresult: u32) {
        let _ = hresult;
    }

    /// The server selected a protocol version outside the range the client offered.
    ///
    /// The channel stays unusable until the server opens it again: Windows App closes it at this point, but a dynamic
    /// channel processor cannot close its own channel, so this one ignores all further traffic instead.
    fn version_negotiation_failed(&mut self, version: ProtocolVersion) {
        let _ = version;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    AwaitingVersion {
        channel_id: u32,
    },
    Active {
        channel_id: u32,
        version: ProtocolVersion,
    },
    /// The server selected a version outside the offered range.
    ///
    /// Windows App closes the channel at this point; the processor cannot close it, so it ignores all further traffic.
    Failed,
}

/// Client processor for `Microsoft::Windows::RDS::VirtualDesktopControl::Switch`.
pub struct VdcSwitchClient {
    launch_uri: LaunchUri,
    handler: Box<dyn VdcSwitchHandler>,
    state: State,
}

impl core::fmt::Debug for VdcSwitchClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VdcSwitchClient")
            .field("launch_uri", &self.launch_uri)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl VdcSwitchClient {
    /// Creates a processor that sends `launch_uri` in its version request.
    pub fn new(launch_uri: LaunchUri, handler: Box<dyn VdcSwitchHandler>) -> Self {
        Self {
            launch_uri,
            handler,
            state: State::Closed,
        }
    }

    /// Returns the channel ID while the channel is open and usable.
    fn open_channel_id(&self) -> Option<u32> {
        match self.state {
            State::AwaitingVersion { channel_id } | State::Active { channel_id, .. } => Some(channel_id),
            State::Closed | State::Failed => None,
        }
    }

    /// Returns the version the server selected, once negotiation has succeeded.
    pub fn negotiated_version(&self) -> Option<ProtocolVersion> {
        match self.state {
            State::Active { version, .. } => Some(version),
            State::Closed | State::AwaitingVersion { .. } | State::Failed => None,
        }
    }

    /// Encodes a switch-to-cloud request, sent when the user switches from the local desktop back to the Cloud PC.
    ///
    /// Like Windows App, this needs only an open channel, not a completed version negotiation.
    pub fn request_switch_to_cloud(&self) -> PduResult<Vec<SvcMessage>> {
        let Some(channel_id) = self.open_channel_id() else {
            return Err(pdu_other_err!(
                "VdcSwitchClient::request_switch_to_cloud",
                "virtual desktop control channel is not open"
            ));
        };

        encode_dvc_messages(
            channel_id,
            vec![Box::new(VdcSwitchPdu::SwitchToCloudRequest)],
            ChannelFlags::empty(),
        )
        .map_err(|error| encode_err!(error))
    }
}

impl_as_any!(VdcSwitchClient);

impl DvcProcessor for VdcSwitchClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.state = State::AwaitingVersion { channel_id };
        debug!(channel_id, "Virtual desktop control channel started");
        Ok(vec![Box::new(VdcSwitchPdu::VersionRequest(VersionRequestPdu::new(
            self.launch_uri.clone(),
        )))])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let Some(channel_id) = self.open_channel_id() else {
            return Ok(Vec::new());
        };

        let pdu: VdcSwitchPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed virtual desktop control PDU");
                return Ok(Vec::new());
            }
        };

        match pdu {
            VdcSwitchPdu::VersionResponse { version } if version.is_supported() => {
                debug!(version = version.0, "Virtual desktop control version negotiated");
                self.state = State::Active { channel_id, version };
                Ok(Vec::new())
            }
            VdcSwitchPdu::VersionResponse { version } => {
                warn!(
                    version = version.0,
                    "Server selected an unsupported virtual desktop control version"
                );
                self.state = State::Failed;
                self.handler.version_negotiation_failed(version);
                Ok(Vec::new())
            }
            VdcSwitchPdu::SwitchToLocalRequest => {
                debug!("Cloud PC requested a switch to the local desktop");
                Ok(match self.handler.switch_to_local_requested() {
                    Some(hresult) => vec![Box::new(VdcSwitchPdu::SwitchToLocalResponse { hresult })],
                    None => Vec::new(),
                })
            }
            VdcSwitchPdu::SwitchToCloudResponse { hresult } => {
                debug!(hresult, "Cloud PC answered the switch to the cloud desktop");
                self.handler.switch_to_cloud_completed(hresult);
                Ok(Vec::new())
            }
            VdcSwitchPdu::PerformLocalAction { action } => {
                match self.state {
                    State::Active { version, .. } if ProtocolVersion::V2 <= version => {
                        debug!(?action, "Cloud PC requested a local action");
                        self.handler.perform_local_action(action);
                    }
                    _ => warn!(
                        ?action,
                        "Ignoring a local action received before version 2 was negotiated"
                    ),
                }
                Ok(Vec::new())
            }
            VdcSwitchPdu::VersionRequest(_)
            | VdcSwitchPdu::SwitchToLocalResponse { .. }
            | VdcSwitchPdu::SwitchToCloudRequest => {
                warn!("Ignoring a client-originated virtual desktop control PDU received from the server");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.state = State::Closed;
        debug!("Virtual desktop control channel closed");
    }
}

impl DvcClientProcessor for VdcSwitchClient {}
