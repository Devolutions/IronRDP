use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{decode, encode_vec, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use super::AnnouncedDevices;
use super::io::{FileRedirectorListener, IoUplink, PnpBackend};
use crate::PNPDR_CHANNEL_NAME;
use crate::pdu::info::{
    CAPABILITY_DYNAMIC_DEVICE_ADDITION, DeviceAdditionPdu, DeviceDescription, DeviceRemovalPdu, PnpInfoPdu, VersionPdu,
};

/// Client major version, as Windows sends it ([MS-RDPEPNP] section 2.2.1.2.2).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
const CLIENT_MAJOR_VERSION: u32 = 1;
/// Client minor version, as the [MS-RDPEPNP] section 4.1 sample sends it.
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
const CLIENT_MINOR_VERSION: u32 = 6;

/// The server sends only fixed-size messages on this channel.
const MAX_MESSAGE_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    WaitingServerVersion,
    WaitingServerLogon,
    Ready,
}

/// Client processor for the `PNPDR` channel (PNP Device Info Subprotocol).
///
/// Register it with `DrdynvcClient::with_dynamic_channel` so that a reopened channel announces the
/// devices again.
/// Devices are announced in one Client Device Addition message once the server sends the
/// Authenticated Client message.
#[derive(Debug)]
pub struct PnpInfoClient {
    state: State,
    channel_id: Option<u32>,
    devices: BTreeMap<u32, DeviceDescription>,
    announced: AnnouncedDevices,
}

impl PnpInfoClient {
    /// Creates the processor for the devices to redirect.
    ///
    /// Fails when two devices share a client device ID or a device cannot be encoded.
    pub fn new(devices: Vec<DeviceDescription>) -> PduResult<Self> {
        let mut client = Self {
            state: State::WaitingServerVersion,
            channel_id: None,
            devices: BTreeMap::new(),
            announced: AnnouncedDevices::default(),
        };
        for device in devices {
            client.insert(device)?;
        }
        Ok(client)
    }

    /// Creates the listener for the `FileRedirectorChannel` instances that carry the I/O of these devices.
    ///
    /// The listener refuses CreateFile requests for devices this processor has not announced.
    /// Replies completed outside [`DvcProcessor::process`] are sent through `uplink`.
    pub fn file_redirector_listener(&self, backend: Box<dyn PnpBackend>, uplink: IoUplink) -> FileRedirectorListener {
        FileRedirectorListener::new(backend, Arc::clone(&self.announced), uplink)
    }

    /// Returns whether the server accepts device addition and removal messages.
    pub fn is_ready(&self) -> bool {
        self.state == State::Ready
    }

    /// Adds a device, returning the Client Device Addition message when the channel is ready.
    ///
    /// Before that, the device is announced with the others when the server authenticates the user.
    pub fn add_device(&mut self, device: DeviceDescription) -> PduResult<Vec<SvcMessage>> {
        let client_device_id = device.client_device_id;
        self.insert(device.clone())?;
        if !self.is_ready() {
            return Ok(Vec::new());
        }
        self.announced.lock().insert(client_device_id);
        self.encode(PnpInfoPdu::RedirectDevices(DeviceAdditionPdu { devices: vec![device] }))
    }

    /// Removes a device, returning the Client Device Removal message when the server knows the device.
    pub fn remove_device(&mut self, client_device_id: u32) -> PduResult<Vec<SvcMessage>> {
        if self.devices.remove(&client_device_id).is_none() {
            return Err(pdu_other_err!(
                "PnpInfoClient::remove_device",
                "unknown client device ID"
            ));
        }
        if !self.announced.lock().remove(&client_device_id) {
            return Ok(Vec::new());
        }
        self.encode(PnpInfoPdu::UnredirectDevice(DeviceRemovalPdu { client_device_id }))
    }

    fn insert(&mut self, device: DeviceDescription) -> PduResult<()> {
        // Encoding checks the field constraints that the type cannot express.
        encode_vec(&device).map_err(|error| encode_err!(error))?;
        if self.devices.contains_key(&device.client_device_id) {
            return Err(pdu_other_err!("PnpInfoClient", "duplicate client device ID"));
        }
        self.devices.insert(device.client_device_id, device);
        Ok(())
    }

    fn encode(&self, pdu: PnpInfoPdu) -> PduResult<Vec<SvcMessage>> {
        let channel_id = self
            .channel_id
            .ok_or_else(|| pdu_other_err!("PnpInfoClient", "PNPDR channel is not open"))?;
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty()).map_err(|error| encode_err!(error))
    }
}

impl_as_any!(PnpInfoClient);

impl DvcProcessor for PnpInfoClient {
    fn channel_name(&self) -> &str {
        PNPDR_CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_MESSAGE_SIZE)
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.state = State::WaitingServerVersion;
        self.channel_id = Some(channel_id);
        self.announced.lock().clear();
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu = match decode::<PnpInfoPdu>(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring a malformed PNPDR message");
                return Ok(Vec::new());
            }
        };

        match (self.state, pdu) {
            (State::WaitingServerVersion, PnpInfoPdu::Version(server_version)) => {
                debug!(?server_version, "Received the PNPDR server version");
                self.state = State::WaitingServerLogon;
                Ok(vec![Box::new(PnpInfoPdu::Version(VersionPdu {
                    major_version: CLIENT_MAJOR_VERSION,
                    minor_version: CLIENT_MINOR_VERSION,
                    capabilities: CAPABILITY_DYNAMIC_DEVICE_ADDITION,
                }))])
            }
            (State::WaitingServerLogon, PnpInfoPdu::ServerLogon) => {
                self.state = State::Ready;
                if self.devices.is_empty() {
                    return Ok(Vec::new());
                }
                debug!(count = self.devices.len(), "Announcing PnP devices");
                self.announced.lock().extend(self.devices.keys().copied());
                Ok(vec![Box::new(PnpInfoPdu::RedirectDevices(DeviceAdditionPdu {
                    devices: self.devices.values().cloned().collect(),
                }))])
            }
            (state, pdu) => {
                warn!(?state, ?pdu, "Ignoring an unexpected PNPDR message");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.state = State::WaitingServerVersion;
        self.channel_id = None;
        self.announced.lock().clear();
    }
}

impl DvcClientProcessor for PnpInfoClient {}
