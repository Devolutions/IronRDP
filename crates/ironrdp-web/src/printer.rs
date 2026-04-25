//! Browser-side virtual printer backend for RDPDR.
//!
//! Architecture mirrors the clipboard backend ([`crate::clipboard`]):
//!
//! * [`WasmPrinterBackend`] lives on the SVC processor side and implements
//!   [`ironrdp::rdpdr::backend::RdpdrBackend`]. It is `Send` (required by the
//!   trait) and holds only per-handle byte buffers plus an mpsc proxy — no
//!   JS callbacks.
//! * [`WasmPrinter`] lives in the session event loop and owns the
//!   `js_sys::Function` callbacks. Per-job completion messages flow from the
//!   backend to the event loop via [`PrinterBackendMessage`].
//!
//! The IRP completion responses (DR_CREATE_RSP / DR_WRITE_RSP / DR_CLOSE_RSP)
//! are synthesised synchronously inside the backend — the RDP peer tracks
//! outstanding IRPs by `completion_id`, so completions just need to get
//! queued onto the SVC out-stream; they don't need JS roundtrips. Each
//! response is wrapped in its matching [`RdpdrPdu`] variant so the
//! [`SvcMessage`] layer prepends the correct RDPDR `SharedHeader`
//! (`RDPDR_CTYP_CORE` + `PAKID_CORE_DEVICE_IOCOMPLETION`) automatically.

use std::collections::HashMap;

use futures_channel::mpsc;
use ironrdp::rdpdr::backend::RdpdrBackend;
use ironrdp::rdpdr::pdu::RdpdrPdu;
use ironrdp::rdpdr::pdu::efs::{
    DeviceCloseResponse, DeviceControlRequest, DeviceCreateResponse, DeviceIoResponse, DeviceWriteResponse,
    Information, NtStatus, PrinterIoRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use ironrdp::rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcMessage;
use tracing::{debug, error, trace, warn};
use wasm_bindgen::prelude::*;

use crate::session::RdpInputEvent;

/// Messages sent from the printer backend to the session event loop.
#[derive(Debug)]
pub(crate) enum PrinterBackendMessage {
    /// A print job's file handle was closed by the server; the accumulated
    /// bytes are the complete document (XPS, PDF, PCL — whatever the server
    /// drove into the virtual printer). The event loop fires the registered
    /// JS `on_job_complete` callback with these bytes.
    JobComplete {
        #[expect(dead_code, reason = "retained for diagnostics / future per-handle correlation")]
        file_id: u32,
        document_bytes: Vec<u8>,
    },
}

/// mpsc proxy used by the backend to hand completed jobs to the event loop.
#[derive(Debug, Clone)]
pub(crate) struct WasmPrinterMessageProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl WasmPrinterMessageProxy {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self { tx }
    }

    fn send(&self, message: PrinterBackendMessage) {
        if self.tx.unbounded_send(RdpInputEvent::Printer(message)).is_err() {
            error!("Failed to queue printer backend message, event loop receiver is closed");
        }
    }
}

/// RDPDR backend that buffers a server-initiated print job in memory and
/// hands the finished document off to the session event loop on
/// `IRP_MJ_CLOSE`.
#[derive(Debug)]
pub(crate) struct WasmPrinterBackend {
    /// Per-file-handle document byte buffer. Populated on `IRP_MJ_CREATE`,
    /// appended by `IRP_MJ_WRITE`, drained on `IRP_MJ_CLOSE`.
    open_files: HashMap<u32, Vec<u8>>,
    /// Monotonic file id counter. The server doesn't care what we stamp
    /// into `DR_CREATE_RSP::FileId` as long as it's unique per open handle
    /// and we echo it back on subsequent Write/Close IRPs.
    next_file_id: u32,
    proxy: WasmPrinterMessageProxy,
}

impl_as_any!(WasmPrinterBackend);

impl WasmPrinterBackend {
    pub(crate) fn new(proxy: WasmPrinterMessageProxy) -> Self {
        Self {
            open_files: HashMap::new(),
            next_file_id: 1,
            proxy,
        }
    }

    fn allocate_file_id(&mut self) -> u32 {
        let id = self.next_file_id;
        self.next_file_id = self.next_file_id.wrapping_add(1);
        if self.next_file_id == 0 {
            self.next_file_id = 1;
        }
        id
    }
}

impl RdpdrBackend for WasmPrinterBackend {
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        // Surface server-side rejection at `warn!` so silent failures
        // (where a redirected device never appears in the session) are
        // visible at the default tracing level.
        if pdu.result_code == NtStatus::SUCCESS {
            debug!(device_id = pdu.device_id, "RDPDR device announce accepted by server");
        } else {
            warn!(
                device_id = pdu.device_id,
                result_code = ?pdu.result_code,
                "RDPDR device announce rejected by server; redirected device will not appear in session"
            );
        }
        Ok(())
    }

    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        warn!("Smartcard IOCTL reached printer-only backend; ignoring");
        Ok(())
    }

    fn handle_drive_io_request(&mut self, _req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        warn!("Drive IRP reached printer-only backend; ignoring");
        Ok(Vec::new())
    }

    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        match req {
            PrinterIoRequest::Create(create) => {
                let file_id = self.allocate_file_id();
                self.open_files.insert(file_id, Vec::new());
                trace!(file_id, path = %create.path, "IRP_MJ_CREATE: opened print handle");

                let response = DeviceCreateResponse {
                    device_io_reply: DeviceIoResponse::new(create.device_io_request, NtStatus::SUCCESS),
                    file_id,
                    // A virtual printer is conceptually opened fresh every time;
                    // the bridge's former implementation used FILE_OPENED and
                    // Windows' own redirector accepts either value.
                    information: Information::FILE_OPENED,
                };
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(response))])
            }
            PrinterIoRequest::Write(write) => {
                let file_id = write.device_io_request.file_id;
                // INVARIANT: write.write_data was decoded via a u32-length-prefixed
                // wire field (MS-RDPEFS 2.2.1.4.4 DR_WRITE_REQ Length), so its
                // in-memory Vec length always round-trips back to a u32.
                let data_len =
                    u32::try_from(write.write_data.len()).expect("write length round-trips from u32 wire decode");

                let io_status = if let Some(buf) = self.open_files.get_mut(&file_id) {
                    buf.extend_from_slice(&write.write_data);
                    trace!(file_id, chunk = data_len, total = buf.len(), "IRP_MJ_WRITE: appended");
                    NtStatus::SUCCESS
                } else {
                    warn!(file_id, "IRP_MJ_WRITE for unknown file_id; rejecting");
                    NtStatus::UNSUCCESSFUL
                };

                let response = DeviceWriteResponse {
                    device_io_reply: DeviceIoResponse::new(write.device_io_request, io_status),
                    length: if io_status == NtStatus::SUCCESS { data_len } else { 0 },
                };
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(response))])
            }
            PrinterIoRequest::Close(close) => {
                let file_id = close.device_io_request.file_id;
                let io_status = if let Some(document_bytes) = self.open_files.remove(&file_id) {
                    debug!(
                        file_id,
                        bytes = document_bytes.len(),
                        "IRP_MJ_CLOSE: handing job to event loop"
                    );
                    self.proxy.send(PrinterBackendMessage::JobComplete {
                        file_id,
                        document_bytes,
                    });
                    NtStatus::SUCCESS
                } else {
                    warn!(file_id, "IRP_MJ_CLOSE for unknown file_id; no job to deliver");
                    NtStatus::UNSUCCESSFUL
                };

                let response = DeviceCloseResponse {
                    device_io_response: DeviceIoResponse::new(close.device_io_request, io_status),
                };
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(response))])
            }
            PrinterIoRequest::Unsupported(req) => {
                debug!(
                    major = ?req.major_function,
                    minor = ?req.minor_function,
                    file_id = req.file_id,
                    completion_id = req.completion_id,
                    "Unsupported printer IRP; responding STATUS_NOT_IMPLEMENTED"
                );
                // `RdpdrPdu` has no bare `DeviceIoResponse` variant, so we
                // reuse `DeviceCloseResponse` — its wire shape is
                // `DeviceIoResponse` + 4 bytes of zero padding, which is
                // wire-compatible with any IRP completion when io_status is
                // a failure code (per MS-RDPEFS 2.2.1.5 the server ignores
                // the trailing parameters body on failure).
                let response = DeviceCloseResponse {
                    device_io_response: DeviceIoResponse::new(req, NtStatus::NOT_IMPLEMENTED),
                };
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(response))])
            }
        }
    }
}

/// Event-loop-side companion to [`WasmPrinterBackend`]. Owns the
/// `js_sys::Function` callbacks (`!Send`, so they live here, not in the
/// backend). The session event loop forwards every
/// [`PrinterBackendMessage`] into [`WasmPrinter::process_message`].
#[derive(Debug)]
pub(crate) struct WasmPrinter {
    callbacks: JsPrinterCallbacks,
}

#[derive(Debug, Clone)]
pub(crate) struct JsPrinterCallbacks {
    /// `function(document_bytes: Uint8Array): void`
    pub(crate) on_job_complete: js_sys::Function,
}

impl WasmPrinter {
    pub(crate) fn new(callbacks: JsPrinterCallbacks) -> Self {
        Self { callbacks }
    }

    pub(crate) fn process_message(&self, message: PrinterBackendMessage) {
        match message {
            PrinterBackendMessage::JobComplete {
                file_id: _,
                document_bytes,
            } => {
                // Hand the bytes to JS as a Uint8Array. js_sys::Uint8Array::from
                // copies the slice into the WASM→JS boundary; the backend's
                // buffer is already consumed at this point.
                let array = js_sys::Uint8Array::from(document_bytes.as_slice());
                let this = JsValue::NULL;
                if let Err(err) = self.callbacks.on_job_complete.call1(&this, &array) {
                    error!(?err, "on_job_complete JS callback threw");
                }
            }
        }
    }
}

/// Factory used by [`crate::session::SessionBuilder::connect`] to build a
/// matched (backend, event-loop) pair from a single mpsc channel.
pub(crate) fn wasm_printer_pair(
    input_events_tx: mpsc::UnboundedSender<RdpInputEvent>,
    on_job_complete: js_sys::Function,
) -> (WasmPrinterBackend, WasmPrinter) {
    let proxy = WasmPrinterMessageProxy::new(input_events_tx);
    let backend = WasmPrinterBackend::new(proxy);
    let printer = WasmPrinter::new(JsPrinterCallbacks { on_job_complete });
    (backend, printer)
}
