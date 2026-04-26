#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(clippy::arithmetic_side_effects)] // FIXME: remove

use ironrdp_core::{ReadCursor, decode_cursor, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use pdu::RdpdrPdu;
use pdu::efs::{
    Capabilities, ClientDeviceListAnnounce, ClientDeviceListRemove, ClientNameRequest, ClientNameRequestUnicodeFlag,
    CoreCapability, CoreCapabilityKind, DEFAULT_PRINTER_DRIVER_NAME, DeviceAnnounceHeader, DeviceControlRequest,
    DeviceIoRequest, DeviceType, Devices, PrinterIoRequest, ServerDeviceAnnounceResponse, VersionAndIdPdu,
    VersionAndIdPduKind,
};
use pdu::esc::{ScardCall, ScardIoCtlCode};
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
    post_logon_devices_announced: bool,
    backend: Box<dyn RdpdrBackend>,
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
            post_logon_devices_announced: false,
            backend,
        }
    }

    #[must_use]
    pub fn with_smartcard(mut self, device_id: u32) -> Self {
        self.capabilities.add_smartcard();
        self.device_list.add_smartcard(device_id);
        self
    }

    /// Adds drive redirection capability.
    ///
    /// Callers may also include `initial_drives` to pre-configure the list of drives to announce to the server.
    /// Note that drives do not need to be pre-configured in order to be redirected, a new drive can be announced
    /// at any time during a session by calling [`Self::add_drive`].
    #[must_use]
    pub fn with_drives(mut self, initial_drives: Option<Vec<(u32, String)>>) -> Self {
        self.capabilities.add_drive();
        if let Some(initial_drives) = initial_drives {
            for (device_id, path) in initial_drives {
                self.device_list.add_drive(device_id, path);
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
        self
    }

    /// Users should call this method to announce a new drive to the server. It's the caller's responsibility
    /// to take the returned [`ClientDeviceListAnnounce`] and send it to the server.
    pub fn add_drive(&mut self, device_id: u32, name: String) -> ClientDeviceListAnnounce {
        self.device_list.add_drive(device_id, name.clone());
        ClientDeviceListAnnounce::new_drive(device_id, name)
    }

    pub fn remove_device(&mut self, device_id: u32) -> Option<ClientDeviceListRemove> {
        Some(ClientDeviceListRemove::remove_device(
            self.device_list.remove_device(device_id)?,
        ))
    }

    pub fn downcast_backend<T: RdpdrBackend>(&self) -> Option<&T> {
        self.backend.as_any().downcast_ref::<T>()
    }

    pub fn downcast_backend_mut<T: RdpdrBackend>(&mut self) -> Option<&mut T> {
        self.backend.as_any_mut().downcast_mut::<T>()
    }

    fn handle_server_announce(&mut self, req: VersionAndIdPdu) -> PduResult<Vec<SvcMessage>> {
        let client_announce_reply =
            RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu::new_client_announce_reply(req).map_err(|e| decode_err!(e))?);
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

    fn handle_server_capability(&mut self, _req: CoreCapability) -> PduResult<Vec<SvcMessage>> {
        let client_capability_response =
            RdpdrPdu::CoreCapability(CoreCapability::new_response(self.capabilities.clone_inner()));
        trace!("sending {:?}", client_capability_response);
        Ok(vec![SvcMessage::from(client_capability_response)])
    }

    fn handle_client_id_confirm(&mut self) -> PduResult<Vec<SvcMessage>> {
        let device_list = self
            .device_list
            .clone_inner()
            .into_iter()
            .filter(Self::is_pre_logon_device)
            .collect::<Vec<_>>();

        if device_list.is_empty() {
            return Ok(Vec::new());
        }

        Self::announce_devices(device_list)
    }

    fn handle_user_loggedon(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.post_logon_devices_announced {
            return Ok(Vec::new());
        }

        self.post_logon_devices_announced = true;

        let device_list = self
            .device_list
            .clone_inner()
            .into_iter()
            .filter(|device| !Self::is_pre_logon_device(device))
            .collect::<Vec<_>>();

        if device_list.is_empty() {
            return Ok(Vec::new());
        }

        Self::announce_devices(device_list)
    }

    fn is_pre_logon_device(device: &DeviceAnnounceHeader) -> bool {
        matches!(device.device_type(), DeviceType::Smartcard)
    }

    fn announce_devices(device_list: Vec<DeviceAnnounceHeader>) -> PduResult<Vec<SvcMessage>> {
        let res = RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce { device_list });
        trace!("sending {:?}", res);
        Ok(vec![SvcMessage::from(res)])
    }

    fn handle_server_device_announce_response(
        &mut self,
        pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<Vec<SvcMessage>> {
        self.backend.handle_server_device_announce_response(pdu)?;
        Ok(Vec::new())
    }

    fn handle_device_io_request(
        &mut self,
        dev_io_req: DeviceIoRequest,
        src: &mut ReadCursor<'_>,
    ) -> PduResult<Vec<SvcMessage>> {
        match self
            .device_list
            .for_device_type(dev_io_req.device_id)
            .map_err(|e| decode_err!(e))?
        {
            DeviceType::Smartcard => {
                let req =
                    DeviceControlRequest::<ScardIoCtlCode>::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                let call = ScardCall::decode(req.io_control_code, src).map_err(|e| decode_err!(e))?;

                debug!(?req);
                debug!(?req.io_control_code, ?call);

                self.backend.handle_scard_call(req, call)?;

                Ok(Vec::new())
            }
            DeviceType::Filesystem => {
                let req = ServerDriveIoRequest::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;

                debug!(?req);

                Ok(self.backend.handle_drive_io_request(req)?)
            }
            DeviceType::Print => {
                let req = PrinterIoRequest::decode(dev_io_req, src).map_err(|e| decode_err!(e))?;
                debug!(?req, "dispatching printer IRP to backend");
                self.backend.handle_printer_io_request(req)
            }
            _ => {
                // This should never happen, as we only announce devices that we support.
                warn!(?dev_io_req, "received packet for unsupported device type");
                Ok(Vec::new())
            }
        }
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
        let pdu = decode_cursor::<RdpdrPdu>(&mut src).map_err(|e| decode_err!(e))?;
        debug!("Received {:?}", pdu);

        match pdu {
            RdpdrPdu::VersionAndIdPdu(pdu) if pdu.kind == VersionAndIdPduKind::ServerAnnounceRequest => {
                self.handle_server_announce(pdu)
            }
            RdpdrPdu::CoreCapability(pdu) if pdu.kind == CoreCapabilityKind::ServerCoreCapabilityRequest => {
                self.handle_server_capability(pdu)
            }
            RdpdrPdu::VersionAndIdPdu(pdu) if pdu.kind == VersionAndIdPduKind::ServerClientIdConfirm => {
                self.handle_client_id_confirm()
            }
            RdpdrPdu::ServerDeviceAnnounceResponse(pdu) => self.handle_server_device_announce_response(pdu),
            RdpdrPdu::DeviceIoRequest(pdu) => self.handle_device_io_request(pdu, &mut src),
            RdpdrPdu::UserLoggedon => self.handle_user_loggedon(),
            // Log-and-drop unrecognised PacketIds (e.g. PrnCacheData,
            // PrnUsingXps); see [`RdpdrPdu::Unhandled`] for rationale.
            RdpdrPdu::Unhandled(packet_id) => {
                warn!(?packet_id, "Ignoring unhandled RDPDR PacketId");
                Ok(vec![])
            }
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
            | RdpdrPdu::DeviceCloseResponse(_)
            | RdpdrPdu::ClientDriveQueryDirectoryResponse(_)
            | RdpdrPdu::ClientDriveQueryVolumeInformationResponse(_)
            | RdpdrPdu::DeviceReadResponse(_)
            | RdpdrPdu::DeviceWriteResponse(_)
            | RdpdrPdu::ClientDriveSetInformationResponse(_)
            | RdpdrPdu::EmptyResponse => Err(pdu_other_err!("Rdpdr", "received unexpected packet")),
        }
    }
}

impl SvcClientProcessor for Rdpdr {}
