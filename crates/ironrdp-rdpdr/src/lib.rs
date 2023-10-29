#![doc = include_str!("../README.md")]
#![allow(clippy::arithmetic_side_effects)] // FIXME: remove
#![allow(clippy::cast_lossless)] // FIXME: remove
#![allow(clippy::cast_possible_truncation)] // FIXME: remove
#![allow(clippy::cast_possible_wrap)] // FIXME: remove
#![allow(clippy::cast_sign_loss)] // FIXME: remove

#[macro_use]
extern crate tracing;

pub mod backend;
pub mod pdu;
use crate::pdu::efs::FilesystemRequest;
pub use backend::{noop::NoopRdpdrBackend, RdpdrBackend};
use ironrdp_pdu::{cursor::ReadCursor, decode_cursor, gcc::ChannelName, other_err, PduResult};
use ironrdp_svc::{impl_as_any, CompressionCondition, StaticVirtualChannelProcessor, SvcMessage};
use pdu::efs::{
    Capabilities, ClientDeviceListAnnounce, ClientNameRequest, ClientNameRequestUnicodeFlag, CoreCapability,
    CoreCapabilityKind, DeviceControlRequest, DeviceIoRequest, DeviceType, Devices, ServerDeviceAnnounceResponse,
    VersionAndIdPdu, VersionAndIdPduKind,
};
use pdu::esc::{ScardCall, ScardIoCtlCode};
use pdu::RdpdrPdu;

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
    /// TODO: explain what this is
    computer_name: String,
    capabilities: Capabilities,
    /// Pre-configured list of devices to announce to the server.
    ///
    /// All devices not of the type [`DeviceType::Filesystem`] must be declared here.
    device_list: Devices,
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

    /// Users should call this method to announce a new drive to the server. It's the caller's responsibility
    /// to take the returned [`ClientDeviceListAnnounce`] and send it to the server.
    pub fn add_drive(&mut self, device_id: u32, name: String) -> ClientDeviceListAnnounce {
        self.device_list.add_drive(device_id, name.clone());
        ClientDeviceListAnnounce::new_drive(device_id, name)
    }

    pub fn downcast_backend<T: RdpdrBackend>(&self) -> Option<&T> {
        self.backend.as_any().downcast_ref::<T>()
    }

    pub fn downcast_backend_mut<T: RdpdrBackend>(&mut self) -> Option<&mut T> {
        self.backend.as_any_mut().downcast_mut::<T>()
    }

    fn handle_server_announce(&mut self, req: VersionAndIdPdu) -> PduResult<Vec<SvcMessage>> {
        let client_announce_reply = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu::new_client_announce_reply(req)?);
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
        let res = RdpdrPdu::CoreCapability(CoreCapability::new_response(self.capabilities.clone_inner()));
        trace!("sending {:?}", res);
        Ok(vec![SvcMessage::from(res)])
    }

    fn handle_client_id_confirm(&mut self) -> PduResult<Vec<SvcMessage>> {
        let res = RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce {
            device_list: self.device_list.clone_inner(),
        });
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
        match self.device_list.for_device_type(dev_io_req.device_id)? {
            DeviceType::Smartcard => {
                let req = DeviceControlRequest::<ScardIoCtlCode>::decode(dev_io_req, src)?;
                let call = ScardCall::decode(req.io_control_code, src)?;

                debug!(?req);
                debug!(?req.io_control_code, ?call);

                self.backend.handle_scard_call(req, call)?;

                Ok(Vec::new())
            }
            DeviceType::Filesystem => {
                let req = FilesystemRequest::decode(dev_io_req, src)?;

                debug!(?req);

                self.backend.handle_fs_request(req)?;

                Ok(Vec::new())
            }
            _ => {
                // This should never happen, as we only announce devices that we support.
                warn!(?dev_io_req, "received packet for unsupported device type");
                Ok(Vec::new())
            }
        }
    }
}

impl StaticVirtualChannelProcessor for Rdpdr {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let mut payload = ReadCursor::new(payload);
        let pdu = decode_cursor::<RdpdrPdu>(&mut payload)?;
        debug!("received {:?}", pdu);

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
            RdpdrPdu::DeviceIoRequest(pdu) => self.handle_device_io_request(pdu, &mut payload),
            RdpdrPdu::Unimplemented => {
                warn!(?pdu, "received unimplemented packet");
                Ok(Vec::new())
            }
            // TODO: This can eventually become a `_ => {}` block, but being explicit for now
            // to make sure we don't miss handling new RdpdrPdu variants here during active development.
            RdpdrPdu::ClientNameRequest(_)
            | RdpdrPdu::ClientDeviceListAnnounce(_)
            | RdpdrPdu::VersionAndIdPdu(_)
            | RdpdrPdu::CoreCapability(_)
            | RdpdrPdu::DeviceControlResponse(_)
            | RdpdrPdu::DeviceCreateResponse(_) => Err(other_err!("Rdpdr", "received unexpected packet")),
        }
    }
}
