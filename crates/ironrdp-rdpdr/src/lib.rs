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
use tracing::{debug, trace, warn};

pub mod backend;
pub mod pdu;
pub mod server;

pub use self::backend::noop::NoopRdpdrBackend;
pub use self::backend::{
    RdpdrBackend, RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive,
};
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
    client_id: Option<u32>,
    server_capabilities_received: bool,
    client_id_confirmed: bool,
    post_logon_devices_announced: bool,
    pending_device_announcements: Vec<u32>,
    pending_drive_removals: Vec<u32>,
    manually_announced_device_ids: Vec<u32>,
    backend_active_drive_ids: Vec<u32>,
    active_device_ids: Vec<u32>,
    rejected_device_ids: Vec<u32>,
    backend: Option<Box<dyn RdpdrBackend>>,
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
            client_id: None,
            server_capabilities_received: false,
            client_id_confirmed: false,
            post_logon_devices_announced: false,
            pending_device_announcements: Vec::new(),
            pending_drive_removals: Vec::new(),
            manually_announced_device_ids: Vec::new(),
            backend_active_drive_ids: Vec::new(),
            active_device_ids: Vec::new(),
            rejected_device_ids: Vec::new(),
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

    /// Returns whether this channel can accept dynamic filesystem devices.
    pub fn drive_hotplug_available(&self) -> bool {
        self.drive_capability_configured
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
        self.pending_device_announcements.push(device_id);
        self.manually_announced_device_ids.push(device_id);
        ClientDeviceListAnnounce::new_drive(device_id, name)
    }

    /// Activates a filesystem device and, when allowed by the current sequence,
    /// announces it to the server.
    ///
    /// [MS-RDPEFS section 3.2.5.1.9] permits later announcements containing only previously unannounced devices.
    ///
    /// [MS-RDPEFS section 3.2.5.1.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5
    pub fn add_dynamic_drive(&mut self, device_id: u32, name: String) -> PduResult<Vec<SvcMessage>> {
        if name.is_empty() || name.contains('\0') {
            return Err(pdu_other_err!("dynamic drive name must be nonempty and contain no NUL"));
        }
        if let Some((_, device_type)) = self.device_types.iter().find(|(id, _)| *id == device_id) {
            if *device_type != DeviceType::Filesystem {
                return Err(pdu_other_err!("dynamic drive uses an already-live device ID"));
            }
            if self.pending_drive_removals.contains(&device_id) {
                self.pending_drive_removals.retain(|id| *id != device_id);
                return Ok(Vec::new());
            }
            if self.rejected_device_ids.contains(&device_id) {
                self.unregister_drive(device_id)?;
            } else {
                return Ok(Vec::new());
            }
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
        self.backend_active_drive_ids.push(device_id);
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
    ///
    /// [MS-RDPEFS section 3.2.5.2.2] requires post-connect removal and invalidates later requests for that device ID.
    ///
    /// [MS-RDPEFS section 3.2.5.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5
    pub fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        if !self
            .device_types
            .iter()
            .any(|(id, device_type)| *id == device_id && *device_type == DeviceType::Filesystem)
        {
            return Err(pdu_other_err!("device removal requires a filesystem device"));
        }
        if self.pending_device_announcements.contains(&device_id) {
            if !self.pending_drive_removals.contains(&device_id) {
                self.pending_drive_removals.push(device_id);
            }
            return Ok(Vec::new());
        }

        let was_active = self.active_device_ids.contains(&device_id);
        let mut messages = if self.backend_active_drive_ids.contains(&device_id) {
            self.backend
                .as_mut()
                .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                .remove_drive(device_id)?
        } else {
            Vec::new()
        };

        self.unregister_drive(device_id)?;

        if was_active {
            messages.push(SvcMessage::from(RdpdrPdu::ClientDeviceListRemove(
                ClientDeviceListRemove::remove_device(device_id),
            )));
        }

        Ok(messages)
    }

    fn unregister_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.device_list
            .remove_device(device_id)
            .ok_or_else(|| pdu_other_err!("device disappeared from the RDPDR device list"))?;
        self.device_types.retain(|(id, _)| *id != device_id);
        self.pending_device_announcements.retain(|id| *id != device_id);
        self.pending_drive_removals.retain(|id| *id != device_id);
        self.manually_announced_device_ids.retain(|id| *id != device_id);
        self.active_device_ids.retain(|id| *id != device_id);
        self.rejected_device_ids.retain(|id| *id != device_id);
        self.backend_active_drive_ids.retain(|id| *id != device_id);
        Ok(())
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
        self.pending_drive_removals.retain(|id| *id != device_id);
        self.manually_announced_device_ids.retain(|id| *id != device_id);
        self.active_device_ids.retain(|id| *id != device_id);
        self.rejected_device_ids.retain(|id| *id != device_id);
        self.backend_active_drive_ids.retain(|id| *id != device_id);
        Some(ClientDeviceListRemove::remove_device(device_id))
    }

    pub fn downcast_backend<T: RdpdrBackend>(&self) -> Option<&T> {
        self.backend.as_ref()?.as_any().downcast_ref::<T>()
    }

    pub fn downcast_backend_mut<T: RdpdrBackend>(&mut self) -> Option<&mut T> {
        self.backend.as_mut()?.as_any_mut().downcast_mut::<T>()
    }

    fn handle_server_announce(&mut self, req: VersionAndIdPdu) -> PduResult<Vec<SvcMessage>> {
        for device_id in core::mem::take(&mut self.pending_drive_removals) {
            self.unregister_drive(device_id)?;
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
            for device_id in &configured_drive_ids {
                backend.restore_drive(*device_id)?;
            }
        }
        self.backend_active_drive_ids = configured_drive_ids;

        self.server_capabilities_received = false;
        self.client_id_confirmed = false;
        self.post_logon_devices_announced = false;
        self.pending_device_announcements.clear();
        self.manually_announced_device_ids.clear();
        self.active_device_ids.clear();
        self.rejected_device_ids.clear();

        let client_id = if req.version_minor < VERSION_MINOR_12 {
            let mut bytes = [0; size_of::<u32>()];
            getrandom::fill(&mut bytes)
                .map_err(|err| pdu_other_err!("generating rdpdr legacy client id", source: err))?;
            u32::from_le_bytes(bytes)
        } else {
            req.client_id
        };
        self.client_id = Some(client_id);
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
        if self.server_capabilities_received {
            return Err(pdu_other_err!("received duplicate RDPDR server capability request"));
        }

        self.server_capabilities_received = true;
        let client_capability_response =
            RdpdrPdu::CoreCapability(CoreCapability::new_response(self.capabilities.clone_supported_by(&req)));
        trace!("sending {:?}", client_capability_response);
        Ok(vec![SvcMessage::from(client_capability_response)])
    }

    fn handle_client_id_confirm(&mut self, req: VersionAndIdPdu) -> PduResult<Vec<SvcMessage>> {
        let expected_client_id = self
            .client_id
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
        self.backend
            .as_mut()
            .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
            .handle_server_device_announce_response(pdu)?;

        if accepted {
            self.pending_device_announcements.retain(|id| *id != device_id);
            if !self.active_device_ids.contains(&device_id) {
                self.active_device_ids.push(device_id);
            }
            self.rejected_device_ids.retain(|id| *id != device_id);
            if self.pending_drive_removals.contains(&device_id) {
                return self.remove_drive(device_id);
            }
            return Ok(Vec::new());
        }

        let mut messages = if self.backend_active_drive_ids.contains(&device_id) {
            self.backend
                .as_mut()
                .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                .remove_drive(device_id)?
        } else {
            Vec::new()
        };
        self.backend_active_drive_ids.retain(|id| *id != device_id);
        self.pending_device_announcements.retain(|id| *id != device_id);
        let removal_requested = self.pending_drive_removals.contains(&device_id);
        self.pending_drive_removals.retain(|id| *id != device_id);
        if !self.rejected_device_ids.contains(&device_id) {
            self.rejected_device_ids.push(device_id);
        }
        self.active_device_ids.retain(|id| *id != device_id);
        if removal_requested {
            self.unregister_drive(device_id)?;
        }

        messages.push(SvcMessage::from(RdpdrPdu::ClientDeviceListRemove(
            ClientDeviceListRemove::remove_device(device_id),
        )));
        Ok(messages)
    }

    fn handle_user_logged_on(&mut self) -> PduResult<Vec<SvcMessage>> {
        if !self.client_id_confirmed {
            return Err(pdu_other_err!(
                "received RDPDR user logged on before client ID confirmation"
            ));
        }

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
        self.backend
            .as_mut()
            .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
            .poll_deferred_messages()
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
            return Ok(vec![]);
        };

        match device_type {
            DeviceType::Smartcard => {
                let req =
                    DeviceControlRequest::<ScardIoCtlCode>::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                let call = ScardCall::decode(req.io_control_code, src).map_err(|e| decode_err!(e))?;

                debug!(?req);
                debug!(?req.io_control_code, ?call);

                self.backend
                    .as_mut()
                    .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                    .handle_scard_call(req, call)
            }
            DeviceType::Filesystem => {
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
                if dev_io_req.major_function == MajorFunction::DeviceControl {
                    let req = DeviceControlRequest::<AnyIoCtlCode>::decode_with_input_buffer(dev_io_req, src)
                        .map_err(|e| decode_err!(e))?;
                    if !src.is_empty() {
                        return Err(pdu_other_err!(
                            "received filesystem device-control IRP with trailing RDPDR data"
                        ));
                    }

                    debug!(?req, "Dispatching filesystem device-control IRP");
                    return self
                        .backend
                        .as_mut()
                        .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                        .handle_drive_device_control(req);
                }
                if dev_io_req.major_function == MajorFunction::Close {
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

                debug!(?req);

                Ok(self
                    .backend
                    .as_mut()
                    .ok_or_else(|| pdu_other_err!("missing rdpdr backend"))?
                    .handle_drive_io_request(req)?)
            }
            DeviceType::Print => match dev_io_req.major_function {
                MajorFunction::DeviceControl => {
                    let req =
                        DeviceControlRequest::<AnyIoCtlCode>::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                    debug!(?req, "Completing printer device-control IRP");

                    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(
                        DeviceControlResponse::new(req, NtStatus::SUCCESS, None),
                    ))])
                }
                MajorFunction::Create | MajorFunction::Write | MajorFunction::Close => {
                    let req = PrinterIoRequest::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                    debug!(?req, "Dispatching printer IRP to backend");
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
            | RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(_)
            | RdpdrPdu::ClientDriveQueryVolumeInformationResponse(_)
            | RdpdrPdu::DeviceReadResponse(_)
            | RdpdrPdu::DeviceWriteResponse(_)
            | RdpdrPdu::DeviceFlushBuffersResponse(_)
            | RdpdrPdu::ClientDriveSetInformationResponse(_)
            | RdpdrPdu::ClientDriveSetSecurityResponse(_)
            | RdpdrPdu::ClientDriveLockControlResponse(_)
            | RdpdrPdu::EmptyResponse => Err(pdu_other_err!("Rdpdr", "received unexpected packet")),
        }
    }
}

impl SvcClientProcessor for Rdpdr {}

#[cfg(test)]
mod tests {
    use ironrdp_core::encode_vec;

    use super::*;

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

        fn handle_scard_call(
            &mut self,
            _req: DeviceControlRequest<ScardIoCtlCode>,
            _call: ScardCall,
        ) -> PduResult<Vec<SvcMessage>> {
            Ok(Vec::new())
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

    #[test]
    fn legacy_server_announce_generates_and_validates_a_client_id() {
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

        let client_id = rdpdr.client_id.expect("generated client ID");
        assert!(
            rdpdr
                .handle_client_id_confirm(VersionAndIdPdu {
                    client_id: client_id.wrapping_add(1),
                    kind: VersionAndIdPduKind::ServerClientIdConfirm,
                    ..server_announce
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
        assert!(
            rdpdr
                .remove_drive(42)
                .expect("defer removal until announcement response")
                .is_empty()
        );
        assert_eq!(
            rdpdr
                .process(&encoded_server_device_announce_response(42, NtStatus::SUCCESS))
                .expect("accept then remove dynamic drive")
                .len(),
            1
        );
        assert!(rdpdr.remove_drive(42).is_err());
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

        assert_eq!(
            rdpdr
                .add_dynamic_drive(42, "C:".to_owned())
                .expect("readd dynamically removed drive")
                .len(),
            1
        );
        assert!(
            rdpdr
                .process(&encoded_server_device_announce_response(42, NtStatus::SUCCESS))
                .expect("accept readded dynamic drive")
                .is_empty()
        );
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .added_drives,
            vec![42, 42]
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
    fn rapid_add_remove_add_cancels_pending_removal() {
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

        assert!(rdpdr.remove_drive(42).expect("defer pending removal").is_empty());
        assert!(
            rdpdr
                .add_dynamic_drive(42, "C:".to_owned())
                .expect("cancel pending removal")
                .is_empty()
        );
        assert!(
            rdpdr
                .process(&encoded_server_device_announce_response(42, NtStatus::SUCCESS))
                .expect("accept retained dynamic drive")
                .is_empty()
        );
        assert!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .removed_drives
                .is_empty()
        );
    }

    #[test]
    fn replacement_sequence_applies_pending_drive_removal() {
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
        rdpdr
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
            .expect("process user logged on");
        assert_eq!(
            rdpdr
                .add_dynamic_drive(42, "C:".to_owned())
                .expect("announce dynamic drive")
                .len(),
            1
        );
        assert!(rdpdr.remove_drive(42).expect("defer pending removal").is_empty());

        rdpdr
            .process(&encoded_server_announce(0x5678))
            .expect("process replacement server announce");
        assert!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .restored_drives
                .is_empty()
        );
        assert!(!rdpdr.device_types.iter().any(|(device_id, _)| *device_id == 42));
    }

    #[test]
    fn initial_drive_can_be_removed_and_readded_dynamically() {
        let mut rdpdr = Rdpdr::new(Box::new(TrackingBackend::default()), "test".to_owned())
            .with_drives(Some(vec![(42, "C:".to_owned())]));
        initialize_drive(&mut rdpdr, 42);

        assert_eq!(rdpdr.remove_drive(42).expect("remove initial drive").len(), 1);
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .removed_drives,
            vec![42]
        );

        assert_eq!(
            rdpdr
                .add_dynamic_drive(42, "C:".to_owned())
                .expect("readd drive with its stable ID")
                .len(),
            1
        );
        assert_eq!(
            rdpdr
                .downcast_backend::<TrackingBackend>()
                .expect("tracking backend")
                .added_drives,
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
        assert!(
            rdpdr
                .remove_drive(42)
                .expect("defer removal until rejected announcement")
                .is_empty()
        );

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
        assert!(rdpdr.remove_drive(42).is_err());
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
