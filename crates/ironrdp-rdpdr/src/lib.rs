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
pub use backend::{noop::NoopRdpdrBackend, RdpdrBackend};
use ironrdp_pdu::{cursor::ReadCursor, decode_cursor, gcc::ChannelName, other_err, PduResult};
use ironrdp_svc::{impl_as_any, CompressionCondition, StaticVirtualChannelProcessor, SvcMessage};
use pdu::efs::{
    Capabilities, ClientDeviceListAnnounce, ClientNameRequest, ClientNameRequestUnicodeFlag, CoreCapability,
    CoreCapabilityKind, DeviceControlRequest, DeviceIoRequest, Devices, ServerDeviceAnnounceResponse, VersionAndIdPdu,
    VersionAndIdPduKind,
};
use pdu::esc::{ScardAccessStartedEventCall, ScardIoCtlCode};
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
        self.device_list.add_smartcard(device_id);
        self.capabilities.add_smartcard();
        self
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

    fn handle_server_device_announce_response(&self, pdu: ServerDeviceAnnounceResponse) -> PduResult<Vec<SvcMessage>> {
        self.backend.handle_server_device_announce_response(pdu)?;
        Ok(Vec::new())
    }

    fn handle_device_io_request(
        &self,
        pdu: DeviceIoRequest,
        payload: &mut ReadCursor<'_>,
    ) -> PduResult<Vec<SvcMessage>> {
        if self.is_for_smartcard(&pdu) {
            let req = DeviceControlRequest::<ScardIoCtlCode>::decode(pdu, payload)?;
            match req.io_control_code {
                ScardIoCtlCode::AccessStartedEvent => {
                    let call = ScardAccessStartedEventCall::decode(payload)?;
                    debug!(?req, ?call, "received smartcard ioctl");
                    self.backend.handle_scard_access_started_event_call(req, call)?;
                }
                _ => {
                    warn!(?req, "received unimplemented smartcard ioctl");
                }
            }
            Ok(Vec::new())
        } else {
            Err(other_err!("Rdpdr", "received unexpected packet"))
        }
    }

    fn is_for_smartcard(&self, pdu: &DeviceIoRequest) -> bool {
        self.device_list.is_smartcard(pdu.device_id)
    }
}

impl_as_any!(Rdpdr);

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
            | RdpdrPdu::DeviceControlResponse(_) => Err(other_err!("Rdpdr", "received unexpected packet")),
        }
    }
}
