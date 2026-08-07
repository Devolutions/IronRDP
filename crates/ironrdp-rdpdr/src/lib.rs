#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(clippy::arithmetic_side_effects)] // FIXME: remove

use ironrdp_core::{ReadCursor, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use pdu::efs::{
    AnyIoCtlCode, Capabilities, ClientDeviceListAnnounce, ClientDeviceListRemove, ClientNameRequest,
    ClientNameRequestUnicodeFlag, CoreCapability, CoreCapabilityKind, DEFAULT_PRINTER_DRIVER_NAME,
    DeviceAnnounceHeader, DeviceCloseResponse, DeviceControlRequest, DeviceControlResponse, DeviceIoRequest,
    DeviceIoResponse, DeviceType, Devices, MajorFunction, NtStatus, PrinterIoRequest, ServerDeviceAnnounceResponse,
    VERSION_MINOR_12, VERSION_MINOR_RDP51, VersionAndIdPdu, VersionAndIdPduKind,
};
use pdu::esc::{ScardCall, ScardIoCtlCode};
use pdu::{PacketId, RdpdrPdu, SharedHeader};
use std::collections::HashSet;
use tracing::{debug, trace, warn};

pub mod backend;
pub mod pdu;

pub use self::backend::RdpdrBackend;
pub use self::backend::noop::NoopRdpdrBackend;
use crate::pdu::efs::ServerDriveIoRequest;

/// The RDPDR channel as specified in [\[MS-RDPEFS\]].
///
/// This channel must always be advertised with the "rdpsnd"
/// channel in order for the server to send anything back to it,
/// see: [\[MS-RDPEFS\] Appendix A<1>]
///
/// [\[MS-RDPEFS\]]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5
/// [\[MS-RDPEFS\] Appendix A<1>]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1
#[derive(Debug)]
pub struct Rdpdr {
    /// The name of the computer that is running the client.
    ///
    /// Any directories shared will be displayed by File Explorer
    /// as "`<directory>` on `<computer_name>`".
    computer_name: String,
    capabilities: Capabilities,
    /// Pre-configured list of devices to announce to the server.
    ///
    /// All devices not of the type [`DeviceType::Filesystem`] must be declared here.
    device_list: Devices,
    device_types: Vec<(u32, DeviceType)>,
    drive_capability_configured: bool,
    /// Client ID selected for the current server-announced RDPDR sequence.
    expected_client_id: Option<u32>,
    server_capabilities_received: bool,
    client_id_confirmed: bool,
    post_logon_devices_announced: bool,
    /// Devices for which a list-announce PDU was sent but no server result has
    /// arrived. Such a device cannot be removed yet.
    pending_device_announcements: HashSet<u32>,
    manually_announced_device_ids: Vec<u32>,
    activated_dynamic_drive_ids: Vec<u32>,
    /// Devices accepted by the server and eligible for device-list removal.
    active_device_ids: HashSet<u32>,
    /// Filesystem devices introduced through [`DynamicDriveOperation`].
    dynamic_device_ids: HashSet<u32>,
    /// Devices the host disabled while waiting for server acceptance.
    pending_device_removals: HashSet<u32>,
    /// Filesystem devices rejected by the server in the current sequence.
    rejected_device_ids: HashSet<u32>,
    events: Vec<RdpdrEvent>,
    backend: Option<Box<dyn RdpdrBackend>>,
}

/// A value-free RDPDR lifecycle signal for host diagnostics.
///
/// The protocol processor records these events without performing I/O. Host
/// integrations can drain them and map them to their own bounded diagnostic
/// delivery mechanism.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RdpdrEvent {
    ServerAnnounce,
    ServerCapabilities,
    ClientIdConfirm,
    UserLoggedOn,
    DeviceListAnnounce,
    DeviceAccepted,
    DeviceRejected,
    DynamicDriveAdded,
    /// A dynamically added drive was accepted by the server.
    DynamicDriveAccepted {
        device_id: u32,
    },
    /// A dynamic drive could not be activated locally or was rejected by the server.
    DynamicDriveRejected {
        device_id: u32,
    },
    /// A dynamic drive has been removed from the local backend and device list.
    DynamicDriveRemoved {
        device_id: u32,
    },
    /// A live filesystem drive could not be removed from the local backend.
    DynamicDriveRemovalFailed {
        device_id: u32,
    },
    /// A server filesystem IRP was received.
    DriveIoRequest(MajorFunction),
    /// A server filesystem IRP targeted a device that is no longer announced.
    DriveIoRequestIgnoredUnknownDevice {
        device_id: u32,
    },
    /// A server filesystem IRP targeted a drive rejected during announcement.
    DriveIoRequestIgnoredRejectedDevice {
        device_id: u32,
    },
    /// Value-free details for a filesystem create request.
    DriveCreateRequest {
        desired_access: u32,
        shared_access: u32,
        create_options: u32,
    },
    /// Value-free details for a filesystem metadata query.
    DriveQueryInformationRequest {
        information_class: u32,
    },
    /// Value-free details for a filesystem directory query.
    DriveQueryDirectoryRequest {
        information_class: u32,
        initial_query: bool,
    },
    /// The requested byte count for a filesystem read.
    DriveReadRequest {
        length: u32,
    },
    /// Value-free correlation details for a filesystem read.
    DriveReadRequestCorrelation {
        device_id: u32,
        file_id: u32,
        completion_id: u32,
        offset: u64,
    },
    /// The requested byte count for a filesystem write.
    DriveWriteRequest {
        length: u32,
    },
    /// Value-free details for a filesystem control request.
    DriveDeviceControlRequest {
        io_control_code: u32,
        input_buffer_length: u32,
        output_buffer_length: u32,
    },
    /// An immediate filesystem IRP completion was produced.
    ///
    /// The optional raw NTSTATUS is intended for value-free host diagnostics.
    /// A missing status means the response could not be encoded for inspection.
    DriveIoCompletion {
        major_function: MajorFunction,
        status: Option<u32>,
    },
    /// The encoded size of the final immediate filesystem response PDU.
    DriveIoResponseSize {
        major_function: MajorFunction,
        bytes: Option<u32>,
    },
    /// The static-channel writer completed the encoded filesystem response frame.
    DriveIoResponseWritten {
        bytes: Option<u32>,
    },
    /// A completed backend-owned deferred filesystem IRP.
    ///
    /// The fields contain only the response status and encoded PDU length.
    DriveIoDeferredCompletion {
        status: Option<u32>,
        bytes: Option<u32>,
    },
    /// A filesystem IRP was intentionally deferred for asynchronous completion.
    DriveIoDeferred(MajorFunction),
}

/// A live filesystem-device change requested by an RDPDR host integration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DynamicDriveOperation {
    /// Activate and announce the filesystem device under the provided name.
    Add { device_id: u32, name: String },
    /// Remove a previously accepted filesystem device.
    Remove { device_id: u32 },
}

impl_as_any!(Rdpdr);

impl Rdpdr {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");

    /// Creates a new [`Rdpdr`].
    pub fn new(backend: Box<dyn RdpdrBackend>, computer_name: String) -> Self {
        Self {
            computer_name,
            capabilities: Capabilities::new(),
            device_list: Devices::new(),
            device_types: Vec::new(),
            drive_capability_configured: false,
            expected_client_id: None,
            server_capabilities_received: false,
            client_id_confirmed: false,
            post_logon_devices_announced: false,
            pending_device_announcements: HashSet::new(),
            manually_announced_device_ids: Vec::new(),
            activated_dynamic_drive_ids: Vec::new(),
            active_device_ids: HashSet::new(),
            dynamic_device_ids: HashSet::new(),
            pending_device_removals: HashSet::new(),
            rejected_device_ids: HashSet::new(),
            events: Vec::new(),
            backend: Some(backend),
        }
    }

    #[must_use]
    pub fn with_smartcard(mut self, device_id: u32) -> Self {
        self.capabilities.add_smartcard();
        self.device_list.add_smartcard(device_id);
        self.device_types.push((device_id, DeviceType::Smartcard));
        self
    }

    /// Adds drive redirection capability.
    ///
    /// Callers may also include `initial_drives` to pre-configure the list of drives to announce to the server.
    /// Note that drives do not need to be pre-configured in order to be redirected, a new drive can be announced
    /// at any time during a session by calling [`Self::add_dynamic_drive`].
    #[must_use]
    pub fn with_drives(mut self, initial_drives: Option<Vec<(u32, String)>>) -> Self {
        self.enable_drive_capability();
        if let Some(initial_drives) = initial_drives {
            for (device_id, path) in initial_drives {
                self.device_list.add_drive(device_id, path);
                self.device_types.push((device_id, DeviceType::Filesystem));
            }
        }
        self
    }

    /// Adds printer redirection capability and announces a single
    /// virtual printer under `device_id` with the user-visible name
    /// `print_name`.
    ///
    /// Uses [`DEFAULT_PRINTER_DRIVER_NAME`] as the PostScript driver and
    /// marks the device as the session's default printer — see
    /// [`pdu::efs::Devices::add_printer`] for the rationale. IRPs
    /// targeting this device are dispatched to
    /// [`RdpdrBackend::handle_printer_io_request`].
    #[must_use]
    pub fn with_printer(self, device_id: u32, print_name: String) -> Self {
        self.with_printer_driver(device_id, print_name, DEFAULT_PRINTER_DRIVER_NAME.to_owned())
    }

    /// Adds printer redirection capability with an explicit server-side
    /// printer driver name.
    ///
    /// Use this when the target host needs a driver other than
    /// [`DEFAULT_PRINTER_DRIVER_NAME`] for the redirected printer queue.
    #[must_use]
    pub fn with_printer_driver(mut self, device_id: u32, print_name: String, driver_name: String) -> Self {
        self.capabilities.add_printer();
        self.device_list
            .add_printer_with_driver(device_id, print_name, driver_name);
        self.device_types.push((device_id, DeviceType::Print));
        self
    }

    /// Builds a raw drive announcement for integrations that manage their own
    /// backend state and static-channel delivery.
    ///
    /// New integrations should use [`Self::add_dynamic_drive`] so activation,
    /// protocol sequencing, and channel delivery stay coordinated.
    ///
    /// When this configures drive redirection for the first time, call it before
    /// processing the server core capability request.
    pub fn add_drive(&mut self, device_id: u32, name: String) -> ClientDeviceListAnnounce {
        self.enable_drive_capability();
        self.device_list.add_drive(device_id, name.clone());
        self.device_types.push((device_id, DeviceType::Filesystem));
        self.pending_device_announcements.insert(device_id);
        self.manually_announced_device_ids.push(device_id);
        ClientDeviceListAnnounce::new_drive(device_id, name)
    }

    /// Activates a filesystem device and, when allowed by the current sequence,
    /// announces it to the server.
    pub fn add_dynamic_drive(&mut self, device_id: u32, name: String) -> PduResult<Vec<SvcMessage>> {
        if name.is_empty() || name.contains('\0') {
            return Err(pdu_other_err!("dynamic drive name must be nonempty and contain no NUL"));
        }
        if self.device_types.iter().any(|(id, _)| *id == device_id) {
            return Err(pdu_other_err!("dynamic drive uses an already-live device ID"));
        }
        if !self.drive_capability_configured && self.server_capabilities_received {
            return Err(pdu_other_err!(
                "dynamic drives require capability configuration before server negotiation"
            ));
        }

        self.enable_drive_capability();
        self.backend
            .as_mut()
            .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
            .add_drive(device_id)?;
        self.activated_dynamic_drive_ids.push(device_id);
        self.device_list.add_drive(device_id, name.clone());
        self.device_types.push((device_id, DeviceType::Filesystem));

        if !self.post_logon_devices_announced {
            return Ok(Vec::new());
        }

        self.announce_devices(
            ClientDeviceListAnnounce::new_drive(device_id, name).device_list,
            vec![device_id],
        )
    }

    /// Releases a filesystem device and sends its removal after pending IRP
    /// cancellations have been returned by the backend.
    pub fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        if !self
            .device_types
            .iter()
            .any(|(id, device_type)| *id == device_id && *device_type == DeviceType::Filesystem)
        {
            return Err(pdu_other_err!("device removal requires a filesystem device"));
        }
        if self.pending_device_announcements.contains(&device_id) {
            return Err(pdu_other_err!(
                "device removal requires the server announcement response"
            ));
        }

        let was_active = self.active_device_ids.contains(&device_id);
        let mut messages = if self.activated_dynamic_drive_ids.contains(&device_id) {
            self.backend
                .as_mut()
                .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                .remove_drive(device_id)?
        } else {
            Vec::new()
        };

        self.device_list
            .remove_device(device_id)
            .ok_or_else(|| pdu_other_err!("device disappeared from the RDPDR device list"))?;
        self.device_types.retain(|(id, _)| *id != device_id);
        self.active_device_ids.retain(|id| *id != device_id);
        self.rejected_device_ids.retain(|id| *id != device_id);
        self.manually_announced_device_ids.retain(|id| *id != device_id);
        self.activated_dynamic_drive_ids.retain(|id| *id != device_id);

        if was_active {
            messages.push(SvcMessage::from(RdpdrPdu::ClientDeviceListRemove(
                ClientDeviceListRemove::remove_device(device_id),
            )));
        }

        Ok(messages)
    }

    /// Builds a removal PDU for a non-filesystem or externally managed device.
    ///
    /// Filesystem devices managed through [`Self::add_dynamic_drive`] must use
    /// [`Self::remove_drive`] so the backend can cancel deferred IRPs before
    /// the removal PDU is sent.
    pub fn remove_device(&mut self, device_id: u32) -> Option<ClientDeviceListRemove> {
        let is_externally_managed_drive = self.manually_announced_device_ids.contains(&device_id);
        if self
            .device_types
            .iter()
            .any(|(id, device_type)| *id == device_id && *device_type == DeviceType::Filesystem)
            && !is_externally_managed_drive
        {
            warn!(
                device_id,
                "Ignoring raw filesystem device removal; use remove_drive instead"
            );
            return None;
        }

        let device_id = self.device_list.remove_device(device_id)?;
        if let Some(index) = self.device_types.iter().position(|(id, _)| *id == device_id) {
            self.device_types.remove(index);
        }
        self.pending_device_announcements.retain(|id| *id != device_id);
        self.manually_announced_device_ids.retain(|id| *id != device_id);
        self.active_device_ids.retain(|id| *id != device_id);
        self.rejected_device_ids.retain(|id| *id != device_id);
        self.activated_dynamic_drive_ids.retain(|id| *id != device_id);
        Some(ClientDeviceListRemove::remove_device(device_id))
    }

    /// Applies a live filesystem-device change.
    ///
    /// Adds received before the post-logon announcement point are retained and
    /// announced as part of the normal post-logon device list. A device cannot
    /// be removed while its server announcement response is outstanding.
    pub fn update_dynamic_drive(&mut self, operation: DynamicDriveOperation) -> PduResult<Vec<SvcMessage>> {
        match operation {
            DynamicDriveOperation::Add { device_id, name } => {
                if name.is_empty() || name.contains('\0') {
                    self.events.push(RdpdrEvent::DynamicDriveRejected { device_id });
                    return Err(pdu_other_err!(
                        "RDPDR dynamic drive name must be nonempty and contain no NUL"
                    ));
                }
                if self.device_list.contains_device(device_id) {
                    if self.pending_device_removals.remove(&device_id) {
                        return Ok(Vec::new());
                    }
                    self.events.push(RdpdrEvent::DynamicDriveRejected { device_id });
                    return Err(pdu_other_err!("RDPDR dynamic drive uses an already-live device ID"));
                }
                self.events.push(RdpdrEvent::DynamicDriveAdded);
                let messages = match self.add_dynamic_drive(device_id, name) {
                    Ok(messages) => messages,
                    Err(error) => {
                        self.events.pop();
                        self.events.push(RdpdrEvent::DynamicDriveRejected { device_id });
                        return Err(error);
                    }
                };
                self.dynamic_device_ids.insert(device_id);
                Ok(messages)
            }
            DynamicDriveOperation::Remove { device_id } => {
                if self.pending_device_announcements.contains(&device_id) {
                    self.pending_device_removals.insert(device_id);
                    return Ok(Vec::new());
                }
                if !self.device_list.contains_device(device_id) {
                    self.events.push(RdpdrEvent::DynamicDriveRemovalFailed { device_id });
                    return Err(pdu_other_err!("RDPDR dynamic drive removal targets an unknown device"));
                }
                let messages = if self.activated_dynamic_drive_ids.contains(&device_id) {
                    self.remove_drive(device_id)
                } else {
                    let mut messages = match self
                        .backend
                        .as_mut()
                        .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                        .remove_drive(device_id)
                    {
                        Ok(messages) => messages,
                        Err(error) => {
                            self.events.push(RdpdrEvent::DynamicDriveRemovalFailed { device_id });
                            return Err(error);
                        }
                    };
                    self.device_list
                        .remove_device(device_id)
                        .ok_or_else(|| pdu_other_err!("RDPDR dynamic drive disappeared from the device list"))?;
                    self.device_types.retain(|(id, _)| *id != device_id);
                    self.manually_announced_device_ids.retain(|id| *id != device_id);
                    self.activated_dynamic_drive_ids.retain(|id| *id != device_id);
                    self.rejected_device_ids.remove(&device_id);
                    if self.active_device_ids.remove(&device_id) {
                        messages.push(SvcMessage::from(RdpdrPdu::ClientDeviceListRemove(
                            ClientDeviceListRemove::remove_device(device_id),
                        )));
                    }
                    Ok(messages)
                };
                match messages {
                    Ok(messages) => {
                        self.dynamic_device_ids.remove(&device_id);
                        self.events.push(RdpdrEvent::DynamicDriveRemoved { device_id });
                        Ok(messages)
                    }
                    Err(error) => {
                        self.events.push(RdpdrEvent::DynamicDriveRemovalFailed { device_id });
                        Err(error)
                    }
                }
            }
        }
    }

    pub fn downcast_backend<T: RdpdrBackend>(&self) -> Option<&T> {
        self.backend.as_ref()?.as_any().downcast_ref::<T>()
    }

    pub fn downcast_backend_mut<T: RdpdrBackend>(&mut self) -> Option<&mut T> {
        self.backend.as_mut()?.as_any_mut().downcast_mut::<T>()
    }

    /// Returns and clears lifecycle events accumulated since the last call.
    pub fn take_events(&mut self) -> Vec<RdpdrEvent> {
        core::mem::take(&mut self.events)
    }

    fn handle_server_announce(&mut self, req: VersionAndIdPdu) -> PduResult<Vec<SvcMessage>> {
        let pending_device_removals = core::mem::take(&mut self.pending_device_removals);
        for device_id in pending_device_removals {
            self.device_list
                .remove_device(device_id)
                .ok_or_else(|| pdu_other_err!("removed RDPDR device is missing from the device list"))?;
            self.device_types.retain(|(id, _)| *id != device_id);
            self.manually_announced_device_ids.retain(|id| *id != device_id);
            self.activated_dynamic_drive_ids.retain(|id| *id != device_id);
            self.active_device_ids.remove(&device_id);
            self.dynamic_device_ids.remove(&device_id);
            self.rejected_device_ids.remove(&device_id);
            self.events.push(RdpdrEvent::DynamicDriveRemoved { device_id });
        }
        let configured_drive_ids = self
            .device_list
            .clone_inner()
            .into_iter()
            .zip(self.device_types.iter().copied())
            .filter_map(|(device, (device_id, _))| {
                (device.device_type() == DeviceType::Filesystem).then_some(device_id)
            })
            .collect::<Vec<_>>();
        {
            let backend = self
                .backend
                .as_mut()
                .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?;
            backend.reset()?;
            for device_id in configured_drive_ids {
                backend.restore_drive(device_id)?;
            }
        }

        self.server_capabilities_received = false;
        self.client_id_confirmed = false;
        self.post_logon_devices_announced = false;
        self.pending_device_announcements.clear();
        self.manually_announced_device_ids.clear();
        self.active_device_ids.clear();
        self.rejected_device_ids.clear();
        self.expected_client_id = None;
        self.events.push(RdpdrEvent::ServerAnnounce);

        let client_id = if req.version_minor < VERSION_MINOR_12 {
            let mut bytes = [0; size_of::<u32>()];
            getrandom::fill(&mut bytes)
                .map_err(|err| pdu_other_err!("generating rdpdr legacy client id", source: err))?;
            u32::from_le_bytes(bytes)
        } else {
            req.client_id
        };
        self.expected_client_id = Some(client_id);
        let client_announce_reply = RdpdrPdu::VersionAndIdPdu(
            VersionAndIdPdu::new_client_announce_reply_with_legacy_client_id(req, client_id)
                .map_err(|e| decode_err!(e))?,
        );
        trace!("sending {:?}", client_announce_reply);

        let client_name_request = RdpdrPdu::ClientNameRequest(ClientNameRequest::new(
            self.computer_name.clone(),
            ClientNameRequestUnicodeFlag::Unicode,
        ));
        trace!("sending {:?}", client_name_request);

        Ok(vec![
            SvcMessage::from(client_announce_reply),
            SvcMessage::from(client_name_request),
        ])
    }

    fn handle_server_capability(&mut self, req: CoreCapability) -> PduResult<Vec<SvcMessage>> {
        if self.expected_client_id.is_none() {
            return Err(pdu_other_err!(
                "received RDPDR server capability request before server announce"
            ));
        }
        if self.server_capabilities_received {
            return Err(pdu_other_err!("received duplicate RDPDR server capability request"));
        }

        self.server_capabilities_received = true;
        self.events.push(RdpdrEvent::ServerCapabilities);
        let client_capability_response =
            RdpdrPdu::CoreCapability(CoreCapability::new_response(self.capabilities.clone_supported_by(&req)));
        trace!("sending {:?}", client_capability_response);
        Ok(vec![SvcMessage::from(client_capability_response)])
    }

    fn handle_client_id_confirm(&mut self, req: VersionAndIdPdu) -> PduResult<Vec<SvcMessage>> {
        let expected_client_id = self
            .expected_client_id
            .ok_or_else(|| pdu_other_err!("received RDPDR client ID confirm before server announce"))?;
        if expected_client_id != req.client_id {
            return Err(pdu_other_err!(
                "received RDPDR client ID confirm for an unexpected client ID"
            ));
        }
        if self.client_id_confirmed {
            return Err(pdu_other_err!("received duplicate RDPDR client ID confirm"));
        }

        let announce_all_devices = req.version_minor == VERSION_MINOR_RDP51;
        let (device_list, device_ids): (Vec<_>, Vec<_>) = self
            .device_list
            .clone_inner()
            .into_iter()
            .zip(self.device_types.iter().copied())
            .filter(|(device, (device_id, _))| {
                !self.manually_announced_device_ids.contains(device_id)
                    && (announce_all_devices || Self::is_pre_logon_device(device))
            })
            .map(|(device, (device_id, _))| (device, device_id))
            .unzip();

        let messages = if device_list.is_empty() {
            Vec::new()
        } else {
            self.announce_devices(device_list, device_ids)?
        };

        self.client_id_confirmed = true;
        self.post_logon_devices_announced = announce_all_devices;
        self.events.push(RdpdrEvent::ClientIdConfirm);
        Ok(messages)
    }

    fn announce_post_logon_devices(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.post_logon_devices_announced {
            return Ok(Vec::new());
        }

        self.post_logon_devices_announced = true;

        let (device_list, device_ids): (Vec<_>, Vec<_>) = self
            .device_list
            .clone_inner()
            .into_iter()
            .zip(self.device_types.iter().copied())
            .filter(|(device, (device_id, _))| {
                !self.manually_announced_device_ids.contains(device_id) && !Self::is_pre_logon_device(device)
            })
            .map(|(device, (device_id, _))| (device, device_id))
            .unzip();

        if device_list.is_empty() {
            return Ok(Vec::new());
        }

        self.announce_devices(device_list, device_ids)
    }

    fn is_pre_logon_device(device: &DeviceAnnounceHeader) -> bool {
        matches!(device.device_type(), DeviceType::Smartcard)
    }

    fn enable_drive_capability(&mut self) {
        if !self.drive_capability_configured {
            self.capabilities.add_drive();
            if self
                .backend
                .as_ref()
                .is_some_and(|backend| backend.supports_drive_security())
            {
                self.capabilities.add_drive_security();
            }
            self.drive_capability_configured = true;
        }
    }

    fn announce_devices(
        &mut self,
        device_list: Vec<DeviceAnnounceHeader>,
        device_ids: Vec<u32>,
    ) -> PduResult<Vec<SvcMessage>> {
        if device_list.len() != device_ids.len() {
            return Err(pdu_other_err!(
                "device announcement does not match the configured RDPDR device list"
            ));
        }
        for (index, device_id) in device_ids.iter().enumerate() {
            if self.pending_device_announcements.contains(device_id)
                || device_ids[..index].iter().any(|previous| previous == device_id)
            {
                return Err(pdu_other_err!(
                    "device was announced more than once before server acknowledgement"
                ));
            }
        }
        self.pending_device_announcements.extend(device_ids);
        let res = RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce { device_list });
        self.events.push(RdpdrEvent::DeviceListAnnounce);
        trace!("sending {:?}", res);
        Ok(vec![SvcMessage::from(res)])
    }

    fn handle_server_device_announce_response(
        &mut self,
        pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<Vec<SvcMessage>> {
        if !self.pending_device_announcements.contains(&pdu.device_id) {
            return Err(pdu_other_err!(
                "server responded to a device that is not awaiting RDPDR acknowledgement"
            ));
        }

        let device_id = pdu.device_id;
        let accepted = pdu.result_code == NtStatus::SUCCESS;
        let is_source_managed_dynamic_drive = self.dynamic_device_ids.contains(&device_id);
        self.backend
            .as_mut()
            .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
            .handle_server_device_announce_response(pdu)?;

        if accepted {
            self.pending_device_announcements.retain(|id| *id != device_id);
            self.active_device_ids.insert(device_id);
            self.rejected_device_ids.retain(|id| *id != device_id);
            self.events.push(RdpdrEvent::DeviceAccepted);
            if self.pending_device_removals.remove(&device_id) {
                return self.update_dynamic_drive(DynamicDriveOperation::Remove { device_id });
            }
            if is_source_managed_dynamic_drive {
                self.events.push(RdpdrEvent::DynamicDriveAccepted { device_id });
            }
            return Ok(Vec::new());
        }

        let mut messages = if self.activated_dynamic_drive_ids.contains(&device_id) {
            self.backend
                .as_mut()
                .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                .remove_drive(device_id)?
        } else {
            Vec::new()
        };
        self.activated_dynamic_drive_ids.retain(|id| *id != device_id);
        self.pending_device_announcements.retain(|id| *id != device_id);
        self.rejected_device_ids.insert(device_id);
        self.active_device_ids.retain(|id| *id != device_id);
        self.pending_device_removals.remove(&device_id);
        self.events.push(RdpdrEvent::DeviceRejected);

        messages.push(SvcMessage::from(RdpdrPdu::ClientDeviceListRemove(
            ClientDeviceListRemove::remove_device(device_id),
        )));
        if is_source_managed_dynamic_drive {
            self.device_list
                .remove_device(device_id)
                .ok_or_else(|| pdu_other_err!("rejected RDPDR device is missing from the device list"))?;
            self.device_types.retain(|(id, _)| *id != device_id);
            self.manually_announced_device_ids.retain(|id| *id != device_id);
            self.dynamic_device_ids.remove(&device_id);
            self.rejected_device_ids.remove(&device_id);
            self.events.push(RdpdrEvent::DynamicDriveRejected { device_id });
        }
        Ok(messages)
    }

    fn handle_user_logged_on(&mut self) -> PduResult<Vec<SvcMessage>> {
        if !self.client_id_confirmed {
            return Err(pdu_other_err!(
                "received RDPDR user logged on before client ID confirmation"
            ));
        }

        self.events.push(RdpdrEvent::UserLoggedOn);
        let mut backend = self.backend.take().expect("missing rdpdr backend");
        let res = backend.handle_user_logged_on(self);
        self.backend = Some(backend);
        let mut messages = res?;
        messages.extend(self.announce_post_logon_devices()?);
        trace!("sending {:?}", messages);
        Ok(messages)
    }

    /// Drains filesystem IRP completions produced by the backend.
    pub fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        let messages = self
            .backend
            .as_mut()
            .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
            .poll_deferred_messages()?;
        self.events
            .extend(messages.iter().map(event_for_deferred_drive_io_completion));
        Ok(messages)
    }

    fn handle_device_io_request(
        &mut self,
        dev_io_req: DeviceIoRequest,
        src: &mut ReadCursor<'_>,
    ) -> PduResult<Vec<SvcMessage>> {
        let Ok(device_type) = self.device_list.for_device_type(dev_io_req.device_id) else {
            // > If a request is received that contains a DeviceId field that was not announced by the client or has
            // > been removed, the request SHOULD be ignored by the implementation.
            // source: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/9925f2e4-8d5a-4777-a41a-7ba6ef6e8bff
            warn!(
                device_id = dev_io_req.device_id,
                file_id = dev_io_req.file_id,
                completion_id = dev_io_req.completion_id,
                "Ignoring filesystem IRP for an unannounced device"
            );
            self.events.push(RdpdrEvent::DriveIoRequestIgnoredUnknownDevice {
                device_id: dev_io_req.device_id,
            });
            return Ok(vec![]);
        };
        if self.rejected_device_ids.contains(&dev_io_req.device_id) {
            // A server must not issue I/O after rejecting the corresponding
            // device announcement. Discard an invalid request rather than
            // exposing a locally configured but rejected device.
            warn!(
                device_id = dev_io_req.device_id,
                file_id = dev_io_req.file_id,
                completion_id = dev_io_req.completion_id,
                "Ignoring filesystem IRP for a rejected device"
            );
            self.events.push(RdpdrEvent::DriveIoRequestIgnoredRejectedDevice {
                device_id: dev_io_req.device_id,
            });
            return Ok(vec![]);
        }

        match device_type {
            DeviceType::Smartcard => {
                let decoded = DeviceControlRequest::<ScardIoCtlCode>::decode_with_input_buffer(dev_io_req, src)
                    .map_err(|e| decode_err!(e))?;
                let mut input = ReadCursor::new(&decoded.input_buffer);
                let req = decoded.request;
                let call = ScardCall::decode(req.io_control_code, &mut input).map_err(|e| decode_err!(e))?;

                trace!(
                    device_id = req.header.device_id,
                    file_id = req.header.file_id,
                    completion_id = req.header.completion_id,
                    io_control_code = ?req.io_control_code,
                    "Dispatching smart-card device-control IRP"
                );

                self.backend
                    .as_mut()
                    .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                    .handle_scard_call(req, call)?;

                Ok(Vec::new())
            }
            DeviceType::Filesystem => {
                let major_function = dev_io_req.major_function;
                let device_id = dev_io_req.device_id;
                let file_id = dev_io_req.file_id;
                let completion_id = dev_io_req.completion_id;
                self.events.push(RdpdrEvent::DriveIoRequest(major_function));
                if self.rejected_device_ids.contains(&dev_io_req.device_id) {
                    warn!(
                        device_id = dev_io_req.device_id,
                        file_id = dev_io_req.file_id,
                        completion_id = dev_io_req.completion_id,
                        "Ignoring filesystem IRP for a rejected device"
                    );
                    return Ok(Vec::new());
                }
                if !self.active_device_ids.contains(&dev_io_req.device_id) {
                    warn!(
                        device_id = dev_io_req.device_id,
                        file_id = dev_io_req.file_id,
                        completion_id = dev_io_req.completion_id,
                        "Ignoring filesystem IRP before device announcement confirmation"
                    );
                    return Ok(Vec::new());
                }
                if major_function == MajorFunction::DeviceControl {
                    let req = DeviceControlRequest::<AnyIoCtlCode>::decode_with_input_buffer(dev_io_req, src)
                        .map_err(|e| decode_err!(e))?;
                    if !src.is_empty() {
                        return Err(pdu_other_err!(
                            "received filesystem device-control IRP with trailing RDPDR data"
                        ));
                    }

                    self.events.push(RdpdrEvent::DriveDeviceControlRequest {
                        io_control_code: req.request.io_control_code.0,
                        input_buffer_length: req.request.input_buffer_length,
                        output_buffer_length: req.request.output_buffer_length,
                    });
                    trace!(
                        device_id,
                        file_id,
                        completion_id,
                        io_control_code = req.request.io_control_code.0,
                        input_buffer_length = req.request.input_buffer_length,
                        output_buffer_length = req.request.output_buffer_length,
                        "Dispatching filesystem device-control IRP"
                    );
                    let messages = self
                        .backend
                        .as_mut()
                        .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                        .handle_drive_device_control(req);
                    match messages {
                        Ok(messages) => {
                            self.events
                                .push(event_for_drive_io_completion(major_function, &messages));
                            self.events
                                .push(event_for_drive_io_response_size(major_function, &messages));
                            return Ok(messages);
                        }
                        Err(error) => {
                            self.events.push(RdpdrEvent::DriveIoCompletion {
                                major_function,
                                status: None,
                            });
                            return Err(error);
                        }
                    }
                }
                if major_function == MajorFunction::Close {
                    const CLOSE_REQUEST_PADDING_SIZE: usize = 32;

                    if src.len() < CLOSE_REQUEST_PADDING_SIZE {
                        return Err(pdu_other_err!("received truncated filesystem close IRP"));
                    }
                    src.advance(CLOSE_REQUEST_PADDING_SIZE);
                }
                let req = ServerDriveIoRequest::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                if !src.is_empty() {
                    return Err(pdu_other_err!("received filesystem IRP with trailing RDPDR data"));
                }
                if let Some(event) = event_for_drive_io_request_details(&req) {
                    self.events.push(event);
                }
                if let ServerDriveIoRequest::DeviceReadRequest(req) = &req {
                    self.events.push(drive_read_request_correlation_event(req));
                }

                trace!(
                    ?major_function,
                    device_id, file_id, completion_id, "Dispatching filesystem IRP"
                );

                let messages = self
                    .backend
                    .as_mut()
                    .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                    .handle_drive_io_request(req)?;
                let completion_event = event_for_drive_io_completion(major_function, &messages);
                debug!(?completion_event, "Completed filesystem IRP");
                self.events.push(completion_event);
                self.events
                    .push(event_for_drive_io_response_size(major_function, &messages));
                Ok(messages)
            }
            DeviceType::Print => match dev_io_req.major_function {
                MajorFunction::DeviceControl => {
                    let req =
                        DeviceControlRequest::<AnyIoCtlCode>::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                    trace!(
                        device_id = req.header.device_id,
                        file_id = req.header.file_id,
                        completion_id = req.header.completion_id,
                        io_control_code = req.io_control_code.0,
                        "Completing printer device-control IRP"
                    );

                    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(
                        DeviceControlResponse::new(req, NtStatus::SUCCESS, None),
                    ))])
                }

                MajorFunction::Create | MajorFunction::Write | MajorFunction::Close => {
                    let req = PrinterIoRequest::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                    let device_io_request = match &req {
                        PrinterIoRequest::Create(req) => &req.device_io_request,
                        PrinterIoRequest::Write(req) => &req.device_io_request,
                        PrinterIoRequest::Close(req) => &req.device_io_request,
                    };
                    trace!(
                        device_id = device_io_request.device_id,
                        file_id = device_io_request.file_id,
                        completion_id = device_io_request.completion_id,
                        major_function = ?device_io_request.major_function,
                        "Dispatching printer IRP to backend"
                    );
                    self.backend
                        .as_mut()
                        .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                        .handle_printer_io_request(req)
                }
                _ => {
                    debug!(
                        major = ?dev_io_req.major_function,
                        minor = ?dev_io_req.minor_function,
                        file_id = dev_io_req.file_id,
                        completion_id = dev_io_req.completion_id,
                        "Completing unsupported printer IRP"
                    );

                    Ok(vec![Self::unsupported_printer_io_response(dev_io_req)])
                }
            },
            _ => {
                // This should never happen, as we only announce devices that we support.
                warn!(?dev_io_req, "received packet for unsupported device type");
                Ok(Vec::new())
            }
        }
    }

    fn unsupported_printer_io_response(device_io_request: DeviceIoRequest) -> SvcMessage {
        SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
            device_io_response: DeviceIoResponse::new(device_io_request, NtStatus::NOT_SUPPORTED),
        }))
    }

    /// Returns whether the current static-channel response completes an immediate filesystem IRP.
    pub fn has_pending_immediate_drive_io_response(&self) -> bool {
        self.events
            .iter()
            .any(|event| matches!(event, RdpdrEvent::DriveIoCompletion { .. }))
    }

    /// Records successful delivery of the current immediate filesystem response frame.
    pub fn record_immediate_drive_io_response_written(&mut self, bytes: usize) {
        self.events.push(RdpdrEvent::DriveIoResponseWritten {
            bytes: u32::try_from(bytes).ok(),
        });
    }
}

fn event_for_drive_io_completion(major_function: MajorFunction, messages: &[SvcMessage]) -> RdpdrEvent {
    let status = (!messages.is_empty())
        .then_some(messages)
        .and_then(|messages| messages.last())
        .and_then(|message| message.encode_unframed_pdu().ok())
        .and_then(|pdu| {
            pdu.get(12..16)
                .map(|status| u32::from_le_bytes(status.try_into().expect("RDPDR status is exactly four bytes")))
        });

    if messages.is_empty() {
        RdpdrEvent::DriveIoDeferred(major_function)
    } else {
        RdpdrEvent::DriveIoCompletion { major_function, status }
    }
}

fn event_for_drive_io_request_details(req: &ServerDriveIoRequest) -> Option<RdpdrEvent> {
    match req {
        ServerDriveIoRequest::ServerCreateDriveRequest(req) => Some(RdpdrEvent::DriveCreateRequest {
            desired_access: req.desired_access.bits(),
            shared_access: req.shared_access.bits(),
            create_options: req.create_options.bits(),
        }),
        ServerDriveIoRequest::ServerDriveQueryInformationRequest(req) => {
            Some(RdpdrEvent::DriveQueryInformationRequest {
                information_class: u32::from(req.file_info_class_lvl.clone()),
            })
        }
        ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(req) => Some(RdpdrEvent::DriveQueryDirectoryRequest {
            information_class: u32::from(req.file_info_class_lvl.clone()),
            initial_query: req.initial_query != 0,
        }),
        ServerDriveIoRequest::DeviceReadRequest(req) => Some(RdpdrEvent::DriveReadRequest { length: req.length }),
        ServerDriveIoRequest::DeviceWriteRequest(req) => Some(RdpdrEvent::DriveWriteRequest {
            length: u32::try_from(req.write_data.len()).expect("decoded RDPDR write length fits in u32"),
        }),
        _ => None,
    }
}

fn drive_read_request_correlation_event(req: &pdu::efs::DeviceReadRequest) -> RdpdrEvent {
    RdpdrEvent::DriveReadRequestCorrelation {
        device_id: req.device_io_request.device_id,
        file_id: req.device_io_request.file_id,
        completion_id: req.device_io_request.completion_id,
        offset: req.offset,
    }
}

fn event_for_drive_io_response_size(major_function: MajorFunction, messages: &[SvcMessage]) -> RdpdrEvent {
    let bytes = messages
        .last()
        .and_then(|message| message.encode_unframed_pdu().ok())
        .and_then(|pdu| u32::try_from(pdu.len()).ok());

    RdpdrEvent::DriveIoResponseSize { major_function, bytes }
}

fn event_for_deferred_drive_io_completion(message: &SvcMessage) -> RdpdrEvent {
    let pdu = message.encode_unframed_pdu().ok();
    let status = pdu.as_deref().and_then(|pdu| {
        pdu.get(12..16)
            .map(|status| u32::from_le_bytes(status.try_into().expect("RDPDR status is exactly four bytes")))
    });
    let bytes = pdu.and_then(|pdu| u32::try_from(pdu.len()).ok());

    RdpdrEvent::DriveIoDeferredCompletion { status, bytes }
}

impl SvcProcessor for Rdpdr {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let mut src = ReadCursor::new(payload);
        let header = SharedHeader::decode(&mut src).map_err(|e| decode_err!(e))?;
        if matches!(header.packet_id, PacketId::PrnCacheData | PacketId::PrnUsingXps) {
            warn!(
                packet_id = ?header.packet_id,
                "Ignoring unhandled RDPDR printer-cache PDU"
            );
            return Ok(vec![]);
        }

        let pdu = RdpdrPdu::decode_body(header, &mut src).map_err(|e| decode_err!(e))?;
        debug!("Received {:?}", pdu);

        match pdu {
            RdpdrPdu::VersionAndIdPdu(pdu) if pdu.kind == VersionAndIdPduKind::ServerAnnounceRequest => {
                self.handle_server_announce(pdu)
            }
            RdpdrPdu::CoreCapability(pdu) if pdu.kind == CoreCapabilityKind::ServerCoreCapabilityRequest => {
                self.handle_server_capability(pdu)
            }
            RdpdrPdu::VersionAndIdPdu(pdu) if pdu.kind == VersionAndIdPduKind::ServerClientIdConfirm => {
                self.handle_client_id_confirm(pdu)
            }
            RdpdrPdu::ServerDeviceAnnounceResponse(pdu) => self.handle_server_device_announce_response(pdu),
            RdpdrPdu::DeviceIoRequest(pdu) => self.handle_device_io_request(pdu, &mut src),
            RdpdrPdu::UserLoggedon => self.handle_user_logged_on(),
            // TODO: This can eventually become a `_ => {}` block, but being explicit for now
            // to make sure we don't miss handling new RdpdrPdu variants here during active development.
            RdpdrPdu::ClientNameRequest(_)
            | RdpdrPdu::ClientDeviceListAnnounce(_)
            | RdpdrPdu::ClientDeviceListRemove(_)
            | RdpdrPdu::VersionAndIdPdu(_)
            | RdpdrPdu::CoreCapability(_)
            | RdpdrPdu::DeviceControlResponse(_)
            | RdpdrPdu::DeviceCreateResponse(_)
            | RdpdrPdu::ClientDriveQueryInformationResponse(_)
            | RdpdrPdu::ClientDriveQuerySecurityResponse(_)
            | RdpdrPdu::DeviceCloseResponse(_)
            | RdpdrPdu::ClientDriveQueryDirectoryResponse(_)
            | RdpdrPdu::ClientDriveQueryVolumeInformationResponse(_)
            | RdpdrPdu::DeviceReadResponse(_)
            | RdpdrPdu::DeviceWriteResponse(_)
            | RdpdrPdu::DeviceFlushBuffersResponse(_)
            | RdpdrPdu::ClientDriveSetInformationResponse(_)
            | RdpdrPdu::ClientDriveSetSecurityResponse(_)
            | RdpdrPdu::ClientDriveSetVolumeInformationResponse(_)
            | RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(_)
            | RdpdrPdu::ClientDriveLockControlResponse(_)
            | RdpdrPdu::EmptyResponse => Err(pdu_other_err!("Rdpdr", "received unexpected packet")),
        }
    }
}

impl SvcClientProcessor for Rdpdr {}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_core::encode_vec;

    #[derive(Debug, Default)]
    struct TrackingBackend {
        resets: usize,
        restored_drives: Vec<u32>,
        announcement_results: Vec<(u32, NtStatus)>,
        drive_requests: Vec<(u32, u32)>,
        control_inputs: Vec<Vec<u8>>,
        added_drives: Vec<u32>,
        removed_drives: Vec<u32>,
        deferred_messages: Vec<SvcMessage>,
    }

    impl_as_any!(TrackingBackend);

    impl RdpdrBackend for TrackingBackend {
        fn reset(&mut self) -> PduResult<()> {
            self.resets += 1;
            Ok(())
        }

        fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
            self.restored_drives.push(device_id);
            Ok(())
        }

        fn add_drive(&mut self, device_id: u32) -> PduResult<()> {
            self.added_drives.push(device_id);
            Ok(())
        }

        fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
            self.removed_drives.push(device_id);
            Ok(Vec::new())
        }

        fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
            self.announcement_results.push((pdu.device_id, pdu.result_code));
            Ok(())
        }

        fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
            Ok(())
        }

        fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
            let ServerDriveIoRequest::DeviceCloseRequest(req) = req else {
                return Err(pdu_other_err!("test backend received an unexpected filesystem IRP"));
            };
            self.drive_requests
                .push((req.device_io_request.device_id, req.device_io_request.completion_id));

            Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
                DeviceCloseResponse {
                    device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::SUCCESS),
                },
            ))])
        }

        fn handle_drive_device_control(
            &mut self,
            req: pdu::efs::DecodedDeviceControlRequest<AnyIoCtlCode>,
        ) -> PduResult<Vec<SvcMessage>> {
            self.control_inputs.push(req.input_buffer);
            Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(
                DeviceControlResponse::new(req.request, NtStatus::SUCCESS, None),
            ))])
        }

        fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
            Ok(core::mem::take(&mut self.deferred_messages))
        }
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four-byte integer"))
    }

    fn encoded_server_announce(client_id: u32) -> Vec<u8> {
        encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_12,
            client_id,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        }))
        .expect("encode server announce")
    }

    fn encoded_server_client_id_confirm(client_id: u32) -> Vec<u8> {
        encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_12,
            client_id,
            kind: VersionAndIdPduKind::ServerClientIdConfirm,
        }))
        .expect("encode server client ID confirm")
    }

    fn encoded_server_device_announce_response(device_id: u32, result_code: NtStatus) -> Vec<u8> {
        encode_vec(&RdpdrPdu::ServerDeviceAnnounceResponse(ServerDeviceAnnounceResponse {
            device_id,
            result_code,
        }))
        .expect("encode server device announce response")
    }

    fn encoded_drive_io_request(device_id: u32, completion_id: u32, major_function: MajorFunction) -> Vec<u8> {
        let mut request = encode_vec(&RdpdrPdu::DeviceIoRequest(DeviceIoRequest {
            device_id,
            file_id: 1,
            completion_id,
            major_function,
            minor_function: pdu::efs::MinorFunction::from(0),
        }))
        .expect("encode drive I/O request");
        if major_function == MajorFunction::Close {
            request.extend([0; 32]);
        }
        request
    }

    fn initialize_drive(rdpdr: &mut Rdpdr, device_id: u32) {
        let responses = rdpdr
            .process(&encoded_server_announce(0x1234))
            .expect("process server announce");
        let client_id = read_u32(
            &responses[0]
                .encode_unframed_pdu()
                .expect("encode client announce response"),
            8,
        );
        assert_eq!(client_id, 0x1234);
        assert!(
            rdpdr
                .process(&encoded_server_client_id_confirm(client_id))
                .expect("process server client ID confirm")
                .is_empty()
        );
        assert_eq!(
            rdpdr
                .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
                .expect("process user logged on")
                .len(),
            1
        );
        assert!(
            rdpdr
                .process(&encoded_server_device_announce_response(device_id, NtStatus::SUCCESS))
                .expect("process accepted device announcement")
                .is_empty()
        );
    }

    #[derive(Debug, Default)]
    struct DynamicDriveBackend {
        added: Vec<u32>,
        removed: Vec<u32>,
        restored: Vec<u32>,
        resets: usize,
        fail_next_removal: bool,
        removal_completions: Vec<SvcMessage>,
        deferred_messages: Vec<SvcMessage>,
        device_control_input: Option<Vec<u8>>,
    }

    impl_as_any!(DynamicDriveBackend);

    impl RdpdrBackend for DynamicDriveBackend {
        fn reset(&mut self) -> PduResult<()> {
            self.resets += 1;
            Ok(())
        }

        fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
            self.restored.push(device_id);
            Ok(())
        }

        fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
            Ok(())
        }

        fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
            Ok(())
        }

        fn add_drive(&mut self, device_id: u32) -> PduResult<()> {
            self.added.push(device_id);
            Ok(())
        }

        fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
            if self.fail_next_removal {
                self.fail_next_removal = false;
                return Err(pdu_other_err!("test dynamic drive removal failure"));
            }
            self.removed.push(device_id);
            Ok(core::mem::take(&mut self.removal_completions))
        }

        fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
            Ok(core::mem::take(&mut self.deferred_messages))
        }

        fn handle_drive_device_control(
            &mut self,
            req: pdu::efs::DecodedDeviceControlRequest<AnyIoCtlCode>,
        ) -> PduResult<Vec<SvcMessage>> {
            self.device_control_input = Some(req.input_buffer);
            Ok(Vec::new())
        }
    }

    fn server_announce(client_id: u32) -> VersionAndIdPdu {
        VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_12,
            client_id,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        }
    }

    fn server_capability_request() -> CoreCapability {
        CoreCapability {
            capabilities: Vec::new(),
            kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
        }
    }

    #[test]
    fn deferred_backend_messages_are_drained_once() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned());
        rdpdr
            .downcast_backend_mut::<DynamicDriveBackend>()
            .expect("dynamic backend")
            .deferred_messages
            .push(SvcMessage::from(vec![1, 2, 3]));

        assert_eq!(rdpdr.poll_deferred_messages().expect("drain deferred message").len(), 1);
        assert!(
            rdpdr
                .poll_deferred_messages()
                .expect("deferred completion queue is now empty")
                .is_empty()
        );
    }

    #[test]
    fn configured_drives_are_announced_in_one_device_list() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned())
            .with_drives(Some(vec![(1, "C".to_owned()), (2, "Data".to_owned())]));
        let messages = rdpdr
            .announce_devices(rdpdr.device_list.clone_inner(), vec![1, 2])
            .expect("announce configured drives");

        assert_eq!(messages.len(), 1);
        let payload = messages[0].encode_unframed_pdu().expect("device list is encodable");
        assert_eq!(
            u32::from_le_bytes(payload[4..8].try_into().expect("device count is present")),
            2
        );
        assert!(rdpdr.pending_device_announcements.contains(&1));
        assert!(rdpdr.pending_device_announcements.contains(&2));
    }

    #[test]
    fn filesystem_device_control_preserves_its_opaque_input_buffer() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        rdpdr.device_list.add_drive(1, "C:".to_owned());
        rdpdr.active_device_ids.insert(1);
        let input_buffer = [0x11; 16];
        let mut payload = Vec::new();
        payload.extend_from_slice(&16u32.to_le_bytes()); // OutputBufferLength
        payload.extend_from_slice(
            &u32::try_from(input_buffer.len())
                .expect("fixed input buffer length fits in u32")
                .to_le_bytes(),
        ); // InputBufferLength
        payload.extend_from_slice(&0x0009_40CFu32.to_le_bytes()); // FSCTL_QUERY_ALLOCATED_RANGES
        payload.resize(32, 0); // Padding
        payload.extend_from_slice(&input_buffer);

        rdpdr
            .handle_device_io_request(
                DeviceIoRequest {
                    device_id: 1,
                    file_id: 2,
                    completion_id: 3,
                    major_function: MajorFunction::DeviceControl,
                    minor_function: pdu::efs::MinorFunction::from(0),
                },
                &mut ReadCursor::new(&payload),
            )
            .expect("decode filesystem Device Control request");

        assert_eq!(
            rdpdr
                .downcast_backend::<DynamicDriveBackend>()
                .expect("dynamic backend")
                .device_control_input
                .as_deref(),
            Some(input_buffer.as_slice())
        );
    }

    #[test]
    fn dynamic_drive_removal_sends_cancellations_before_device_list_remove() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        rdpdr.device_list.add_drive(1, "C:".to_owned());
        rdpdr.active_device_ids.insert(1);
        rdpdr
            .downcast_backend_mut::<DynamicDriveBackend>()
            .expect("dynamic backend")
            .removal_completions
            .push(SvcMessage::from(vec![0xCA, 0xFE]));

        let messages = rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Remove { device_id: 1 })
            .expect("remove active dynamic drive");

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].encode_unframed_pdu().expect("cancellation is encodable"),
            [0xCA, 0xFE]
        );
        assert_eq!(
            messages[1]
                .encode_unframed_pdu()
                .expect("device list removal is encodable")[..4],
            [0x72, 0x44, 0x4D, 0x44],
            "the removal follows all cancellation completions"
        );
    }

    #[test]
    fn dynamic_drive_lifecycle_waits_for_server_acknowledgement() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);

        let queued = rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Add {
                device_id: 1,
                name: "C:".to_owned(),
            })
            .expect("queue drive before post-logon");
        assert!(queued.is_empty());
        assert_eq!(rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap().added, [1]);
        assert_eq!(rdpdr.take_events(), [RdpdrEvent::DynamicDriveAdded]);

        rdpdr.post_logon_devices_announced = true;
        let announce = rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Add {
                device_id: 2,
                name: "D:".to_owned(),
            })
            .expect("announce drive after post-logon");
        assert_eq!(announce.len(), 1);
        let queued_removal = rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Remove { device_id: 2 })
            .expect("queue removal until the server accepts the drive");
        assert!(queued_removal.is_empty());
        rdpdr
            .downcast_backend_mut::<DynamicDriveBackend>()
            .expect("dynamic backend")
            .removal_completions
            .push(SvcMessage::from(vec![0xCA, 0xFE]));

        let remove = rdpdr
            .handle_server_device_announce_response(ServerDeviceAnnounceResponse {
                device_id: 2,
                result_code: NtStatus::SUCCESS,
            })
            .expect("accept dynamic drive");
        assert_eq!(remove.len(), 2);
        assert_eq!(
            remove[0].encode_unframed_pdu().expect("cancellation is encodable"),
            [0xCA, 0xFE]
        );
        assert_eq!(
            remove[1]
                .encode_unframed_pdu()
                .expect("device list removal is encodable")[..4],
            [0x72, 0x44, 0x4D, 0x44]
        );
        assert_eq!(rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap().removed, [2]);
        assert_eq!(
            rdpdr.take_events(),
            [
                RdpdrEvent::DynamicDriveAdded,
                RdpdrEvent::DeviceListAnnounce,
                RdpdrEvent::DeviceAccepted,
                RdpdrEvent::DynamicDriveRemoved { device_id: 2 },
            ]
        );
    }

    #[test]
    fn readding_a_pending_removed_drive_cancels_its_removal() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        rdpdr.post_logon_devices_announced = true;

        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Add {
                device_id: 1,
                name: "C:".to_owned(),
            })
            .expect("announce dynamic drive");
        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Remove { device_id: 1 })
            .expect("queue removal while waiting for acknowledgement");
        assert!(
            rdpdr
                .update_dynamic_drive(DynamicDriveOperation::Add {
                    device_id: 1,
                    name: "C:".to_owned(),
                })
                .expect("cancel pending removal")
                .is_empty()
        );

        assert!(
            rdpdr
                .handle_server_device_announce_response(ServerDeviceAnnounceResponse {
                    device_id: 1,
                    result_code: NtStatus::SUCCESS,
                })
                .expect("accept dynamic drive")
                .is_empty()
        );
        assert!(rdpdr.active_device_ids.contains(&1));
        assert!(rdpdr.device_list.contains_device(1));
        assert!(rdpdr.dynamic_device_ids.contains(&1));
        assert_eq!(rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap().removed, []);
    }

    #[test]
    fn server_announce_discards_pending_drive_removals() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        rdpdr.post_logon_devices_announced = true;
        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Add {
                device_id: 1,
                name: "C:".to_owned(),
            })
            .expect("announce dynamic drive");
        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Remove { device_id: 1 })
            .expect("queue removal while waiting for acknowledgement");
        rdpdr.take_events();

        rdpdr
            .handle_server_announce(VersionAndIdPdu {
                version_major: 1,
                version_minor: VERSION_MINOR_12,
                client_id: 1,
                kind: VersionAndIdPduKind::ServerAnnounceRequest,
            })
            .expect("restart the RDPDR sequence");

        assert!(!rdpdr.device_list.contains_device(1));
        assert!(!rdpdr.dynamic_device_ids.contains(&1));
        let backend = rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap();
        assert!(backend.removed.is_empty());
        assert_eq!(backend.resets, 1);
        assert_eq!(
            rdpdr.take_events(),
            [
                RdpdrEvent::DynamicDriveRemoved { device_id: 1 },
                RdpdrEvent::ServerAnnounce,
            ]
        );
    }

    #[test]
    fn failed_pending_removal_can_be_retried_after_acceptance() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        rdpdr.post_logon_devices_announced = true;
        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Add {
                device_id: 1,
                name: "C:".to_owned(),
            })
            .expect("announce dynamic drive");
        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Remove { device_id: 1 })
            .expect("queue removal while waiting for acknowledgement");
        rdpdr
            .downcast_backend_mut::<DynamicDriveBackend>()
            .expect("dynamic backend")
            .fail_next_removal = true;

        rdpdr
            .handle_server_device_announce_response(ServerDeviceAnnounceResponse {
                device_id: 1,
                result_code: NtStatus::SUCCESS,
            })
            .expect_err("the first local removal fails");

        assert!(rdpdr.active_device_ids.contains(&1));
        assert!(rdpdr.device_list.contains_device(1));
        assert!(
            rdpdr
                .update_dynamic_drive(DynamicDriveOperation::Remove { device_id: 1 })
                .expect("retry removal after acceptance")
                .last()
                .expect("device-list removal response")
                .encode_unframed_pdu()
                .expect("device-list removal is encodable")
                .starts_with(&[0x72, 0x44, 0x4D, 0x44])
        );
    }

    #[test]
    fn rejected_dynamic_drive_reports_its_device_id() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        rdpdr.post_logon_devices_announced = true;

        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Add {
                device_id: 1,
                name: "C:".to_owned(),
            })
            .expect("announce dynamic drive");
        let removal = rdpdr
            .handle_server_device_announce_response(ServerDeviceAnnounceResponse {
                device_id: 1,
                result_code: NtStatus::ACCESS_DENIED,
            })
            .expect("reject dynamic drive");
        assert_eq!(removal.len(), 1);
        assert!(
            removal[0]
                .encode_unframed_pdu()
                .expect("device-list removal is encodable")
                .starts_with(&[0x72, 0x44, 0x4D, 0x44])
        );
        assert!(!rdpdr.dynamic_device_ids.contains(&1));
        assert_eq!(rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap().removed, [1]);

        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Add {
                device_id: 1,
                name: "C:".to_owned(),
            })
            .expect("reuse the removed drive ID");
        assert_eq!(rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap().added, [1, 1]);

        assert_eq!(
            rdpdr.take_events(),
            [
                RdpdrEvent::DynamicDriveAdded,
                RdpdrEvent::DeviceListAnnounce,
                RdpdrEvent::DeviceRejected,
                RdpdrEvent::DynamicDriveRejected { device_id: 1 },
                RdpdrEvent::DynamicDriveAdded,
                RdpdrEvent::DeviceListAnnounce,
            ]
        );
    }

    #[test]
    fn rejected_preconfigured_drive_is_restored_for_a_new_server_sequence() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned())
            .with_drives(Some(vec![(1, "C:".to_owned())]));
        rdpdr.pending_device_announcements.insert(1);

        let removal = rdpdr
            .handle_server_device_announce_response(ServerDeviceAnnounceResponse {
                device_id: 1,
                result_code: NtStatus::ACCESS_DENIED,
            })
            .expect("reject preconfigured drive");
        assert_eq!(removal.len(), 1);
        assert!(rdpdr.device_list.contains_device(1));
        assert!(rdpdr.rejected_device_ids.contains(&1));

        rdpdr
            .handle_server_announce(VersionAndIdPdu {
                version_major: 1,
                version_minor: VERSION_MINOR_12,
                client_id: 1,
                kind: VersionAndIdPduKind::ServerAnnounceRequest,
            })
            .expect("restart the RDPDR sequence");

        assert!(!rdpdr.rejected_device_ids.contains(&1));
        let backend = rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap();
        assert_eq!(backend.resets, 1);
        assert_eq!(backend.restored, [1]);
    }

    #[test]
    fn device_io_is_discarded_after_the_server_rejects_the_drive() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        rdpdr.device_list.add_drive(1, "C:".to_owned());
        rdpdr.rejected_device_ids.insert(1);
        let payload = [0; 32];
        let request = DeviceIoRequest {
            device_id: 1,
            file_id: 2,
            completion_id: 3,
            major_function: MajorFunction::DeviceControl,
            minor_function: pdu::efs::MinorFunction::from(0),
        };

        assert!(
            rdpdr
                .handle_device_io_request(request, &mut ReadCursor::new(&payload))
                .expect("discard rejected-device request")
                .is_empty()
        );
        assert!(
            rdpdr
                .downcast_backend::<DynamicDriveBackend>()
                .expect("dynamic backend")
                .device_control_input
                .is_none()
        );
        assert_eq!(
            rdpdr.take_events(),
            [RdpdrEvent::DriveIoRequestIgnoredRejectedDevice { device_id: 1 }]
        );
    }

    #[test]
    fn device_io_for_an_unannounced_drive_is_reported_to_the_host() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);
        let request = DeviceIoRequest {
            device_id: 1,
            file_id: 2,
            completion_id: 3,
            major_function: MajorFunction::DeviceControl,
            minor_function: pdu::efs::MinorFunction::from(0),
        };

        assert!(
            rdpdr
                .handle_device_io_request(request, &mut ReadCursor::new(&[]))
                .expect("discard unannounced-device request")
                .is_empty()
        );
        assert_eq!(
            rdpdr.take_events(),
            [RdpdrEvent::DriveIoRequestIgnoredUnknownDevice { device_id: 1 }]
        );
    }

    #[test]
    fn failed_removal_of_a_preconfigured_drive_is_reported_to_the_host() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned())
            .with_drives(Some(vec![(1, "C:".to_owned())]));
        rdpdr.active_device_ids.insert(1);
        rdpdr
            .downcast_backend_mut::<DynamicDriveBackend>()
            .expect("dynamic backend")
            .fail_next_removal = true;

        rdpdr
            .update_dynamic_drive(DynamicDriveOperation::Remove { device_id: 1 })
            .expect_err("report local removal failure");

        assert_eq!(
            rdpdr.take_events(),
            [RdpdrEvent::DynamicDriveRemovalFailed { device_id: 1 }]
        );
    }

    #[test]
    fn dynamic_drive_rejects_empty_or_nul_containing_names_before_activation() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned()).with_drives(None);

        for (device_id, name) in [(1, ""), (2, "C:\0untrusted")] {
            rdpdr
                .update_dynamic_drive(DynamicDriveOperation::Add {
                    device_id,
                    name: name.to_owned(),
                })
                .expect_err("invalid dynamic drive name");
        }

        let backend = rdpdr.downcast_backend::<DynamicDriveBackend>().unwrap();
        assert!(backend.added.is_empty());
        assert_eq!(
            rdpdr.take_events(),
            [
                RdpdrEvent::DynamicDriveRejected { device_id: 1 },
                RdpdrEvent::DynamicDriveRejected { device_id: 2 },
            ]
        );
    }

    #[test]
    fn server_capability_request_requires_a_server_announce() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned());

        assert!(
            rdpdr
                .handle_server_capability(server_capability_request())
                .expect_err("a server capability request before server announce is invalid")
                .to_string()
                .contains("before server announce")
        );
    }

    #[test]
    fn client_id_confirm_requires_the_announced_client_id() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned());
        rdpdr
            .handle_server_announce(server_announce(1))
            .expect("start the RDPDR sequence");
        rdpdr
            .handle_server_capability(server_capability_request())
            .expect("receive server capabilities");

        assert!(
            rdpdr
                .handle_client_id_confirm(VersionAndIdPdu {
                    version_major: 1,
                    version_minor: VERSION_MINOR_12,
                    client_id: 2,
                    kind: VersionAndIdPduKind::ServerClientIdConfirm,
                })
                .expect_err("a mismatched client ID confirm is invalid")
                .to_string()
                .contains("unexpected client ID")
        );
    }

    #[test]
    fn server_capability_request_is_allowed_after_client_id_confirm() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned());
        rdpdr
            .handle_server_announce(server_announce(1))
            .expect("start the RDPDR sequence");

        assert!(
            rdpdr
                .handle_client_id_confirm(VersionAndIdPdu {
                    version_major: 1,
                    version_minor: VERSION_MINOR_12,
                    client_id: 1,
                    kind: VersionAndIdPduKind::ServerClientIdConfirm,
                })
                .expect("confirm the client ID without a capability exchange")
                .is_empty()
        );
        assert!(rdpdr.client_id_confirmed);
        let response = rdpdr
            .handle_server_capability(server_capability_request())
            .expect("receive server capabilities after confirming the client ID");
        assert_eq!(response.len(), 1);
        assert!(rdpdr.server_capabilities_received);
    }

    #[test]
    fn repeated_server_announce_can_omit_a_second_capability_request() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned());
        rdpdr
            .handle_server_announce(server_announce(3))
            .expect("start the initial RDPDR sequence");
        rdpdr
            .handle_server_capability(server_capability_request())
            .expect("receive the initial capability request");
        rdpdr
            .handle_client_id_confirm(VersionAndIdPdu {
                version_major: 1,
                version_minor: VERSION_MINOR_12,
                client_id: 3,
                kind: VersionAndIdPduKind::ServerClientIdConfirm,
            })
            .expect("confirm the initial client ID");

        rdpdr
            .handle_server_announce(server_announce(2))
            .expect("restart the RDPDR sequence");
        assert!(
            rdpdr
                .handle_client_id_confirm(VersionAndIdPdu {
                    version_major: 1,
                    version_minor: VERSION_MINOR_12,
                    client_id: 2,
                    kind: VersionAndIdPduKind::ServerClientIdConfirm,
                })
                .expect("confirm the restarted client ID without another capability request")
                .is_empty()
        );
    }

    #[test]
    fn legacy_client_id_confirm_does_not_require_server_capabilities() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned());
        rdpdr
            .handle_server_announce(VersionAndIdPdu {
                version_major: 1,
                version_minor: 4,
                client_id: 0,
                kind: VersionAndIdPduKind::ServerAnnounceRequest,
            })
            .expect("start a legacy RDPDR sequence");
        let client_id = rdpdr.expected_client_id.expect("generated legacy client ID");

        assert!(
            rdpdr
                .handle_client_id_confirm(VersionAndIdPdu {
                    version_major: 1,
                    version_minor: 4,
                    client_id,
                    kind: VersionAndIdPduKind::ServerClientIdConfirm,
                })
                .expect("legacy confirmation without capabilities")
                .is_empty()
        );
        assert!(rdpdr.client_id_confirmed);
    }

    #[test]
    fn duplicate_client_id_confirm_cannot_reannounce_a_dynamic_drive() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned())
            .with_drives(Some(vec![(1, "C:".to_owned())]));
        rdpdr
            .handle_server_announce(server_announce(1))
            .expect("start the RDPDR sequence");
        rdpdr
            .handle_server_capability(server_capability_request())
            .expect("receive server capabilities");
        let client_id_confirm = VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_RDP51,
            client_id: 1,
            kind: VersionAndIdPduKind::ServerClientIdConfirm,
        };

        assert_eq!(
            rdpdr
                .handle_client_id_confirm(client_id_confirm.clone())
                .expect("confirm the client ID")
                .len(),
            1
        );
        assert!(
            rdpdr
                .handle_client_id_confirm(client_id_confirm)
                .expect_err("a duplicate client ID confirm is invalid")
                .to_string()
                .contains("duplicate RDPDR client ID confirm")
        );
    }

    #[test]
    fn user_logged_on_requires_client_id_confirmation() {
        let mut rdpdr = Rdpdr::new(Box::<DynamicDriveBackend>::default(), "test".to_owned());
        rdpdr
            .handle_server_announce(server_announce(1))
            .expect("start the RDPDR sequence");
        rdpdr
            .handle_server_capability(server_capability_request())
            .expect("receive server capabilities");

        assert!(
            rdpdr
                .handle_user_logged_on()
                .expect_err("a user logged on packet before client ID confirmation is invalid")
                .to_string()
                .contains("before client ID confirmation")
        );
    }

    #[test]
    fn read_request_correlation_tracks_only_protocol_identifiers() {
        let req = pdu::efs::DeviceReadRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id: 2,
                completion_id: 3,
                major_function: MajorFunction::Read,
                minor_function: pdu::efs::MinorFunction::from(0),
            },
            length: 4_096,
            offset: 8_192,
        };

        assert_eq!(
            drive_read_request_correlation_event(&req),
            RdpdrEvent::DriveReadRequestCorrelation {
                device_id: 1,
                file_id: 2,
                completion_id: 3,
                offset: 8_192,
            }
        );
    }

    #[test]
    fn legacy_server_announce_validates_the_generated_client_id() {
        let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "test".to_owned());
        let server_announce = VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_RDP51,
            client_id: 0,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        };

        assert_eq!(
            rdpdr
                .handle_server_announce(server_announce.clone())
                .expect("handle legacy server announce")
                .len(),
            2
        );

        let client_id = rdpdr.expected_client_id.expect("generated client ID");
        assert!(
            rdpdr
                .handle_client_id_confirm(VersionAndIdPdu {
                    client_id: client_id.wrapping_add(1),
                    kind: VersionAndIdPduKind::ServerClientIdConfirm,
                    ..server_announce.clone()
                })
                .is_err()
        );
        assert!(
            rdpdr
                .handle_client_id_confirm(VersionAndIdPdu {
                    client_id,
                    kind: VersionAndIdPduKind::ServerClientIdConfirm,
                    ..server_announce
                })
                .expect("confirm generated client ID")
                .is_empty()
        );
    }

    #[test]
    fn confirmed_drive_irps_dispatch_and_map_backend_completions() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned())
            .with_drives(Some(vec![(42, "C:".to_owned())]));

        initialize_drive(&mut rdpdr, 42);

        let responses = rdpdr
            .process(&encoded_drive_io_request(42, 0x100, MajorFunction::Close))
            .expect("process drive close");
        assert_eq!(responses.len(), 1);
        let encoded = responses[0].encode_unframed_pdu().expect("encode drive completion");
        assert_eq!(read_u32(&encoded, 4), 42);
        assert_eq!(read_u32(&encoded, 8), 0x100);
        assert_eq!(NtStatus::from(read_u32(&encoded, 12)), NtStatus::SUCCESS);

        let backend = rdpdr.downcast_backend::<TrackingBackend>().expect("tracking backend");
        assert_eq!(backend.resets, 1);
        assert_eq!(backend.restored_drives, vec![42]);
        assert_eq!(backend.announcement_results, vec![(42, NtStatus::SUCCESS)]);
        assert_eq!(backend.drive_requests, vec![(42, 0x100)]);

        assert_eq!(
            rdpdr
                .process(&encoded_server_announce(0x5678))
                .expect("process replacement server announce")
                .len(),
            2
        );
        let backend = rdpdr.downcast_backend::<TrackingBackend>().expect("tracking backend");
        assert_eq!(backend.resets, 2);
        assert_eq!(backend.restored_drives, vec![42, 42]);
    }

    #[test]
    fn drive_control_payloads_are_validated_and_dispatched_exactly() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned())
            .with_drives(Some(vec![(42, "C:".to_owned())]));
        initialize_drive(&mut rdpdr, 42);

        let mut request = encoded_drive_io_request(42, 0x101, MajorFunction::DeviceControl);
        request.extend_from_slice(&0u32.to_le_bytes()); // OutputBufferLength
        request.extend_from_slice(&3u32.to_le_bytes()); // InputBufferLength
        request.extend_from_slice(&0x1234u32.to_le_bytes()); // IoControlCode
        request.extend_from_slice(&[0; 20]); // Padding
        request.extend_from_slice(&[1, 2, 3]); // InputBuffer

        let responses = rdpdr.process(&request).expect("process drive device control");
        assert_eq!(responses.len(), 1);
        let encoded = responses[0]
            .encode_unframed_pdu()
            .expect("encode drive control completion");
        assert_eq!(NtStatus::from(read_u32(&encoded, 12)), NtStatus::SUCCESS);
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .control_inputs,
            vec![vec![1, 2, 3]]
        );

        request.push(4);
        assert!(rdpdr.process(&request).is_err());
    }

    #[test]
    fn drive_close_padding_is_consumed_before_dispatch() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned())
            .with_drives(Some(vec![(42, "C:".to_owned())]));
        initialize_drive(&mut rdpdr, 42);

        assert_eq!(
            rdpdr
                .process(&encoded_drive_io_request(42, 0x100, MajorFunction::Close))
                .expect("process padded drive close")
                .len(),
            1
        );

        let mut request = encoded_drive_io_request(42, 0x101, MajorFunction::Close);
        request.push(0);
        assert!(rdpdr.process(&request).is_err());

        let mut request = encoded_drive_io_request(42, 0x102, MajorFunction::Close);
        request.pop();
        assert!(rdpdr.process(&request).is_err());
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .drive_requests,
            vec![(42, 0x100)]
        );
    }

    #[test]
    fn unconfirmed_or_rejected_drive_irps_are_ignored() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned())
            .with_drives(Some(vec![(42, "C:".to_owned())]));
        let responses = rdpdr
            .process(&encoded_server_announce(0x1234))
            .expect("process server announce");
        let client_id = read_u32(
            &responses[0]
                .encode_unframed_pdu()
                .expect("encode client announce response"),
            8,
        );
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .expect("process server client ID confirm");
        rdpdr
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
            .expect("process user logged on");

        assert!(
            rdpdr
                .process(&encoded_drive_io_request(7, 0x100, MajorFunction::Close))
                .expect("process unknown drive request")
                .is_empty()
        );
        let responses = rdpdr
            .process(&encoded_server_device_announce_response(42, NtStatus::ACCESS_DENIED))
            .expect("process rejected device announcement");
        assert_eq!(responses.len(), 1);
        let encoded = responses[0].encode_unframed_pdu().expect("encode device removal");
        assert_eq!(&encoded[..4], &[0x72, 0x44, 0x4d, 0x44]);
        assert!(
            rdpdr
                .process(&encoded_drive_io_request(42, 0x101, MajorFunction::Close))
                .expect("process rejected drive request")
                .is_empty()
        );
        assert!(
            rdpdr
                .process(&encoded_server_device_announce_response(42, NtStatus::ACCESS_DENIED))
                .is_err()
        );
        assert!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .drive_requests
                .is_empty()
        );
    }

    #[test]
    fn unsupported_or_malformed_drive_irps_do_not_reach_the_backend() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned())
            .with_drives(Some(vec![(42, "C:".to_owned())]));
        initialize_drive(&mut rdpdr, 42);

        assert!(
            rdpdr
                .process(&encoded_drive_io_request(
                    42,
                    0x100,
                    MajorFunction::SetVolumeInformation
                ))
                .is_err()
        );
        assert!(
            rdpdr
                .process(&encoded_drive_io_request(42, 0x101, MajorFunction::Read))
                .is_err()
        );
        assert!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .drive_requests
                .is_empty()
        );
    }

    #[test]
    fn dynamic_drive_lifecycle_waits_for_announcement_confirmation() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned());
        assert!(
            rdpdr
                .add_dynamic_drive(42, "C:".to_owned())
                .expect("add dynamic drive")
                .is_empty()
        );
        assert!(rdpdr.drive_capability_configured);

        let responses = rdpdr
            .process(&encoded_server_announce(0x1234))
            .expect("process server announce");
        let client_id = read_u32(
            &responses[0]
                .encode_unframed_pdu()
                .expect("encode client announce response"),
            8,
        );
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .expect("process server client ID confirm");
        assert_eq!(
            rdpdr
                .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
                .expect("process user logged on")
                .len(),
            1
        );
        assert!(rdpdr.remove_drive(42).is_err());
        rdpdr
            .process(&encoded_server_device_announce_response(42, NtStatus::SUCCESS))
            .expect("process accepted device announcement");

        let responses = rdpdr.remove_drive(42).expect("remove active dynamic drive");
        assert_eq!(responses.len(), 1);
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .added_drives,
            vec![42]
        );
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .removed_drives,
            vec![42]
        );
    }

    #[test]
    fn rejected_dynamic_drive_releases_backend_resources() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned());
        rdpdr.add_dynamic_drive(42, "C:".to_owned()).expect("add dynamic drive");

        let responses = rdpdr
            .process(&encoded_server_announce(0x1234))
            .expect("process server announce");
        let client_id = read_u32(
            &responses[0]
                .encode_unframed_pdu()
                .expect("encode client announce response"),
            8,
        );
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .expect("process server client ID confirm");
        rdpdr
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
            .expect("process user logged on");

        assert_eq!(
            rdpdr
                .process(&encoded_server_device_announce_response(42, NtStatus::ACCESS_DENIED))
                .expect("process rejected dynamic drive")
                .len(),
            1
        );
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .removed_drives,
            vec![42]
        );
        assert!(
            rdpdr
                .remove_drive(42)
                .expect("remove rejected dynamic drive")
                .is_empty()
        );
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .removed_drives,
            vec![42]
        );
    }

    #[test]
    fn raw_device_removal_cannot_bypass_dynamic_drive_cleanup() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned());
        rdpdr.add_dynamic_drive(42, "C:".to_owned()).expect("add dynamic drive");

        assert!(rdpdr.remove_device(42).is_none());
        assert!(rdpdr.remove_drive(42).expect("remove dynamic drive").is_empty());
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .removed_drives,
            vec![42]
        );

        let mut raw_rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned());
        raw_rdpdr.add_drive(7, "D:".to_owned());
        assert!(raw_rdpdr.remove_device(7).is_some());
    }

    #[test]
    fn dynamic_drive_requires_capability_configuration_before_negotiation() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned());
        rdpdr.server_capabilities_received = true;

        assert!(rdpdr.add_dynamic_drive(42, "C:".to_owned()).is_err());
        assert!(rdpdr.device_list.clone_inner().is_empty());
    }

    #[test]
    fn raw_drive_announcement_is_confirmed_without_reannouncement() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned());
        let responses = rdpdr
            .process(&encoded_server_announce(0x1234))
            .expect("process server announce");
        let client_id = read_u32(
            &responses[0]
                .encode_unframed_pdu()
                .expect("encode client announce response"),
            8,
        );
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .expect("process server client ID confirm");

        let announcement = rdpdr.add_drive(42, "C:".to_owned());
        assert_eq!(announcement.device_list.len(), 1);
        assert!(
            rdpdr
                .process(&encoded_server_device_announce_response(42, NtStatus::SUCCESS))
                .expect("process raw drive announcement response")
                .is_empty()
        );
        assert!(
            rdpdr
                .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
                .expect("process user logged on")
                .is_empty()
        );
        assert_eq!(
            rdpdr
                .process(&encoded_drive_io_request(42, 0x100, MajorFunction::Close))
                .expect("process drive close")
                .len(),
            1
        );
    }

    #[test]
    fn deferred_drive_completions_are_drained_once() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned());
        rdpdr
            .downcast_backend_mut::<TrackingBackend>()
            .expect("tracking backend")
            .deferred_messages
            .push(SvcMessage::from(RdpdrPdu::EmptyResponse));

        assert_eq!(
            rdpdr
                .poll_deferred_messages()
                .expect("drain deferred drive completion")
                .len(),
            1
        );
        assert!(
            rdpdr
                .poll_deferred_messages()
                .expect("drain empty deferred queue")
                .is_empty()
        );
    }
}
