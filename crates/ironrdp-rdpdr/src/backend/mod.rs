pub mod noop;

use core::fmt;

use ironrdp_core::AsAny;
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_svc::SvcMessage;

use crate::Rdpdr;
use crate::pdu::RdpdrPdu;
use crate::pdu::efs::{
    AnyIoCtlCode, ClientDriveLockControlResponse, ClientDriveNotifyChangeDirectoryResponse,
    ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse, ClientDriveQuerySecurityResponse,
    ClientDriveQueryVolumeInformationResponse, ClientDriveSetInformationResponse, ClientDriveSetSecurityResponse,
    ClientDriveSetVolumeInformationResponse, DecodedDeviceControlRequest, DeviceCloseResponse, DeviceControlRequest,
    DeviceCreateResponse, DeviceIoResponse, DeviceReadResponse, DeviceWriteResponse, Information, NtStatus,
    PrinterIoRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use crate::pdu::esc::{ScardCall, ScardIoCtlCode};

/// OS-specific device redirection backend interface.
pub trait RdpdrBackend: AsAny + fmt::Debug + Send {
    /// Indicates whether the backend implements both RDPDR security IRPs.
    ///
    /// The channel advertises the corresponding optional capability bits only
    /// when this returns `true`.
    fn supports_drive_security(&self) -> bool {
        false
    }

    /// Releases all state owned by the current RDPDR initialization sequence.
    ///
    /// A repeated Server Announce Request begins a new sequence. Implementations
    /// must close local handles and discard pending bookkeeping before the
    /// channel accepts new device announcements.
    fn reset(&mut self) -> PduResult<()> {
        Ok(())
    }

    /// Reopens local state for a filesystem device retained across a new RDPDR
    /// initialization sequence.
    ///
    /// [`Self::reset`] runs first and releases every prior device reference.
    /// The channel then restores each currently configured filesystem device
    /// before it can be announced in the new sequence.
    fn restore_drive(&mut self, _device_id: u32) -> PduResult<()> {
        Ok(())
    }

    /// Processes the server's result for a device announcement.
    ///
    /// On an unsuccessful result, the channel releases any local state created
    /// by [`Self::add_drive`] through [`Self::remove_drive`]. Implementations
    /// can use this hook to record the server result, but must not release the
    /// drive state here.
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_scard_call(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ScardCall) -> PduResult<()>;

    /// Activates local state for a dynamically announced filesystem device.
    ///
    /// The caller has verified that the device ID is not currently live and
    /// will only expose it to the server after this method succeeds.
    fn add_drive(&mut self, _device_id: u32) -> PduResult<()> {
        Err(ironrdp_pdu::pdu_other_err!(
            "dynamic drive activation is not supported by this RDPDR backend"
        ))
    }

    /// Invalidates local state for a dynamically removed filesystem device.
    ///
    /// Implementations must release the device's root, file handles, and any
    /// pending operations before this call returns. Any resulting cancellation
    /// completions must be returned so the caller can send them before the
    /// device-removal message makes later requests invalid.
    fn remove_drive(&mut self, _device_id: u32) -> PduResult<Vec<SvcMessage>> {
        Err(ironrdp_pdu::pdu_other_err!(
            "dynamic drive removal is not supported by this RDPDR backend"
        ))
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        unsupported_drive_io_response(req)
    }

    /// Handles a filesystem Device Control request with its exact opaque input
    /// payload. The default preserves the established generic unsupported
    /// completion behavior for backends that do not implement a control.
    fn handle_drive_device_control(
        &mut self,
        req: DecodedDeviceControlRequest<AnyIoCtlCode>,
    ) -> PduResult<Vec<SvcMessage>> {
        self.handle_drive_io_request(ServerDriveIoRequest::DeviceControlRequest(req.request))
    }

    /// Drains completions for filesystem IRPs that the backend deferred.
    ///
    /// Implementations must only return each completion once. The caller polls
    /// this method from the client session loop and sends the returned messages
    /// through the existing RDPDR static channel.
    fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    fn handle_user_logged_on(&mut self, _rdpdr: &mut Rdpdr) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    /// Handle a server-initiated IRP addressed to a printer device.
    ///
    /// `req` carries the fully-decoded printer IRP. Printers only see
    /// [`PrinterIoRequest::Create`] / [`PrinterIoRequest::Write`] /
    /// [`PrinterIoRequest::Close`] on the backend path. Unsupported printer
    /// major functions are completed by the SVC processor before the backend
    /// is called.
    ///
    /// Return the PDUs to send back on the RDPDR channel —
    /// typically a [`crate::pdu::efs::DeviceIoResponse`]-wrapped
    /// `DeviceCreateResponse` / `DeviceWriteResponse` /
    /// `DeviceCloseResponse`. Returning an empty `Vec` is allowed
    /// when the backend has already queued a response out of band
    /// and/or wants to defer.
    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        let device_io_request = req.into_device_io_request();
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
            DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(device_io_request, NtStatus::NOT_SUPPORTED),
            },
        ))])
    }
}

/// A filesystem device to announce for one RDPDR channel lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdpdrDrive {
    /// The device identifier used in RDPDR requests.
    pub device_id: u32,
    /// The user-visible name sent in the filesystem-device announcement.
    pub name: String,
}

/// Per-connection product created by an [`RdpdrBackendFactory`].
///
/// The backend and drive list must describe the same immutable device set. The
/// client constructs a fresh product for every connection attempt so no file
/// handle state or announcement can cross a reconnect boundary.
pub struct RdpdrBackendProduct {
    /// Backend serving the announced filesystem devices.
    pub backend: Box<dyn RdpdrBackend>,
    /// Filesystem devices to announce when the server accepts RDPDR.
    pub initial_drives: Vec<RdpdrDrive>,
}

impl RdpdrBackendProduct {
    /// Creates a product for one RDPDR channel lifetime.
    pub fn new(backend: Box<dyn RdpdrBackend>, initial_drives: Vec<RdpdrDrive>) -> Self {
        Self {
            backend,
            initial_drives,
        }
    }
}

/// Result returned when creating an RDPDR backend for one connection attempt.
pub type RdpdrBackendFactoryResult<T> = Result<T, Box<dyn core::error::Error + Send + Sync>>;

/// Builds a fresh RDPDR backend and device set for each connection attempt.
///
/// A backend owns per-session device and file-handle state, so sharing it
/// across reconnects would let stale file IDs escape into a new RDPDR
/// sequence.
pub trait RdpdrBackendFactory {
    /// Creates the backend and matching filesystem-device announcements for one
    /// RDPDR channel lifetime.
    ///
    /// An error aborts the connection attempt. Factories must not advertise a
    /// device whose local root or backend state could not be created.
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct>;
}

/// Completes a decoded filesystem IRP with `STATUS_NOT_SUPPORTED`.
///
/// Each RDPDR filesystem request requires the response layout specified for its
/// `MajorFunction`. `IRP_MJ_FLUSH_BUFFERS` is the exception: it uses the
/// generic Device I/O Response layout directly.
pub fn unsupported_drive_io_response(req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
    let response = match req {
        ServerDriveIoRequest::ServerCreateDriveRequest(req) => RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
            file_id: 0,
            information: Information::empty(),
        }),
        ServerDriveIoRequest::ServerDriveQueryInformationRequest(req) => {
            RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
                buffer: None,
            })
        }
        ServerDriveIoRequest::ServerDriveQuerySecurityRequest(req) => {
            RdpdrPdu::ClientDriveQuerySecurityResponse(ClientDriveQuerySecurityResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
                security_descriptor: None,
            })
        }
        ServerDriveIoRequest::DeviceCloseRequest(req) => RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
            device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
        }),
        ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(req) => {
            RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse {
                device_io_reply: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
                buffer: None,
            })
        }
        ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(req) => {
            RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(ClientDriveNotifyChangeDirectoryResponse::new(
                req.device_io_request,
                NtStatus::NOT_SUPPORTED,
                Vec::new(),
            ))
        }
        ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(req) => {
            RdpdrPdu::ClientDriveQueryVolumeInformationResponse(ClientDriveQueryVolumeInformationResponse::new(
                req.device_io_request,
                NtStatus::NOT_SUPPORTED,
                None,
            ))
        }
        ServerDriveIoRequest::ServerDriveSetVolumeInformationRequest(req) => {
            RdpdrPdu::ClientDriveSetVolumeInformationResponse(ClientDriveSetVolumeInformationResponse::new(
                req,
                NtStatus::NOT_SUPPORTED,
            ))
        }
        ServerDriveIoRequest::DeviceControlRequest(req) => RdpdrPdu::DeviceControlResponse(
            crate::pdu::efs::DeviceControlResponse::new(req, NtStatus::NOT_SUPPORTED, None),
        ),
        ServerDriveIoRequest::DeviceReadRequest(req) => RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
            read_data: Vec::new(),
        }),
        ServerDriveIoRequest::DeviceWriteRequest(req) => RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
            length: 0,
        }),
        ServerDriveIoRequest::DeviceFlushBuffersRequest(req) => {
            RdpdrPdu::DeviceFlushBuffersResponse(crate::pdu::efs::DeviceFlushBuffersResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
            })
        }
        ServerDriveIoRequest::ServerDriveSetInformationRequest(req) => RdpdrPdu::ClientDriveSetInformationResponse(
            ClientDriveSetInformationResponse::new(&req, NtStatus::NOT_SUPPORTED).map_err(|e| encode_err!(e))?,
        ),
        ServerDriveIoRequest::ServerDriveSetSecurityRequest(req) => RdpdrPdu::ClientDriveSetSecurityResponse(
            ClientDriveSetSecurityResponse::new(&req, NtStatus::NOT_SUPPORTED).map_err(|e| encode_err!(e))?,
        ),
        ServerDriveIoRequest::ServerDriveLockControlRequest(req) => RdpdrPdu::ClientDriveLockControlResponse(
            ClientDriveLockControlResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
        ),
    };

    Ok(vec![SvcMessage::from(response)])
}
