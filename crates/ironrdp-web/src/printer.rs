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
use std::fmt;
use std::sync::Arc;

use core::sync::atomic::{AtomicUsize, Ordering};

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

/// Maximum in-memory print job accepted by the browser backend.
const MAX_PRINT_JOB_BYTES: usize = 128 * 1024 * 1024; // 128 MiB
/// Maximum completed print-job bytes allowed to wait in the event queue.
const MAX_QUEUED_PRINT_JOB_BYTES: usize = MAX_PRINT_JOB_BYTES;

/// Messages sent from the printer backend to the session event loop.
#[derive(Debug)]
pub(crate) enum PrinterBackendMessage {
    /// A print job's file handle was closed by the server; the accumulated
    /// bytes are the complete document produced by the announced server-side
    /// driver. The default web path uses PostScript and expects the receiver
    /// to convert it before presenting a browser print dialog.
    JobComplete {
        file_id: u32,
        document_bytes: Vec<u8>,
        _queued_bytes: QueuedPrintJobBytes,
    },
}

pub(crate) struct QueuedPrintJobBytes {
    len: usize,
    queued_bytes: Arc<AtomicUsize>,
}

impl Drop for QueuedPrintJobBytes {
    fn drop(&mut self) {
        self.queued_bytes.fetch_sub(self.len, Ordering::AcqRel);
    }
}

impl fmt::Debug for QueuedPrintJobBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueuedPrintJobBytes")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// mpsc proxy used by the backend to hand completed jobs to the event loop.
#[derive(Debug, Clone)]
pub(crate) struct WasmPrinterMessageProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
    queued_job_bytes: Arc<AtomicUsize>,
    queued_job_bytes_limit: usize,
}

impl WasmPrinterMessageProxy {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self::new_with_limit(tx, MAX_QUEUED_PRINT_JOB_BYTES)
    }

    fn send_job_complete(&self, file_id: u32, document_bytes: Vec<u8>) -> bool {
        let Some(queued_bytes) = self.reserve_queue_capacity(document_bytes.len()) else {
            warn!(
                file_id,
                bytes = document_bytes.len(),
                limit = self.queued_job_bytes_limit,
                "Completed print job exceeds queued print job byte budget"
            );
            return false;
        };

        if self
            .tx
            .unbounded_send(RdpInputEvent::Printer(PrinterBackendMessage::JobComplete {
                file_id,
                document_bytes,
                _queued_bytes: queued_bytes,
            }))
            .is_err()
        {
            error!("Failed to queue printer backend message, event loop receiver is closed");
            return false;
        }

        true
    }

    fn reserve_queue_capacity(&self, len: usize) -> Option<QueuedPrintJobBytes> {
        let mut queued = self.queued_job_bytes.load(Ordering::Acquire);
        loop {
            let next = queued.checked_add(len)?;
            if next > self.queued_job_bytes_limit {
                return None;
            }

            match self
                .queued_job_bytes
                .compare_exchange_weak(queued, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    return Some(QueuedPrintJobBytes {
                        len,
                        queued_bytes: Arc::clone(&self.queued_job_bytes),
                    });
                }
                Err(actual) => queued = actual,
            }
        }
    }

    fn new_with_limit(tx: mpsc::UnboundedSender<RdpInputEvent>, queued_job_bytes_limit: usize) -> Self {
        Self {
            tx,
            queued_job_bytes: Arc::new(AtomicUsize::new(0)),
            queued_job_bytes_limit,
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
    max_print_job_bytes: usize,
    proxy: WasmPrinterMessageProxy,
}

impl_as_any!(WasmPrinterBackend);

impl WasmPrinterBackend {
    pub(crate) fn new(proxy: WasmPrinterMessageProxy) -> Self {
        Self::new_with_limit(proxy, MAX_PRINT_JOB_BYTES)
    }

    fn new_with_limit(proxy: WasmPrinterMessageProxy, max_print_job_bytes: usize) -> Self {
        Self {
            open_files: HashMap::new(),
            next_file_id: 1,
            max_print_job_bytes,
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

                let mut drop_partial_job = false;
                let io_status = match self.open_files.get_mut(&file_id) {
                    Some(buf) => {
                        let projected_len = buf.len().checked_add(write.write_data.len());
                        if projected_len.is_some_and(|len| len <= self.max_print_job_bytes) {
                            buf.extend_from_slice(&write.write_data);
                            trace!(file_id, chunk = data_len, total = buf.len(), "IRP_MJ_WRITE: appended");
                            NtStatus::SUCCESS
                        } else {
                            warn!(
                                file_id,
                                chunk = data_len,
                                current = buf.len(),
                                limit = self.max_print_job_bytes,
                                "IRP_MJ_WRITE exceeds print job size limit; rejecting and dropping partial job"
                            );
                            drop_partial_job = true;
                            NtStatus::UNSUCCESSFUL
                        }
                    }
                    None => {
                        warn!(file_id, "IRP_MJ_WRITE for unknown file_id; rejecting");
                        NtStatus::UNSUCCESSFUL
                    }
                };
                if drop_partial_job {
                    self.open_files.remove(&file_id);
                }

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
                    if self.proxy.send_job_complete(file_id, document_bytes) {
                        NtStatus::SUCCESS
                    } else {
                        NtStatus::UNSUCCESSFUL
                    }
                } else {
                    warn!(file_id, "IRP_MJ_CLOSE for unknown file_id; no job to deliver");
                    NtStatus::UNSUCCESSFUL
                };

                let response = DeviceCloseResponse {
                    device_io_response: DeviceIoResponse::new(close.device_io_request, io_status),
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
                file_id,
                document_bytes,
                _queued_bytes: _,
            } => {
                // Hand the bytes to JS as a Uint8Array. The conversion copies
                // the Rust buffer, so peak memory can temporarily include both
                // the Rust buffer and the JS array for one job.
                trace!(
                    file_id,
                    bytes = document_bytes.len(),
                    "Delivering print job to JS callback"
                );
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp::rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest, DeviceIoRequest,
        DeviceWriteRequest, FileAttributes, MajorFunction, MinorFunction, SharedAccess,
    };

    const DEVICE_ID: u32 = 42;

    fn printer_backend() -> (WasmPrinterBackend, mpsc::UnboundedReceiver<RdpInputEvent>) {
        printer_backend_with_limits(MAX_PRINT_JOB_BYTES, MAX_QUEUED_PRINT_JOB_BYTES)
    }

    fn printer_backend_with_limits(
        max_print_job_bytes: usize,
        max_queued_print_job_bytes: usize,
    ) -> (WasmPrinterBackend, mpsc::UnboundedReceiver<RdpInputEvent>) {
        let (tx, rx) = mpsc::unbounded();
        let proxy = WasmPrinterMessageProxy::new_with_limit(tx, max_queued_print_job_bytes);
        (WasmPrinterBackend::new_with_limit(proxy, max_print_job_bytes), rx)
    }

    fn device_io_request(file_id: u32, completion_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: DEVICE_ID,
            file_id,
            completion_id,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

    fn create_request(completion_id: u32) -> DeviceCreateRequest {
        DeviceCreateRequest {
            device_io_request: device_io_request(0, completion_id, MajorFunction::Create),
            desired_access: DesiredAccess::empty(),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: String::new(),
        }
    }

    fn write_request(file_id: u32, completion_id: u32, write_data: Vec<u8>) -> DeviceWriteRequest {
        DeviceWriteRequest {
            device_io_request: device_io_request(file_id, completion_id, MajorFunction::Write),
            offset: 0,
            write_data,
        }
    }

    fn close_request(file_id: u32, completion_id: u32) -> DeviceCloseRequest {
        DeviceCloseRequest {
            device_io_request: device_io_request(file_id, completion_id, MajorFunction::Close),
        }
    }

    fn response_bytes(messages: Vec<SvcMessage>) -> Vec<u8> {
        assert_eq!(messages.len(), 1);
        messages[0].encode_unframed_pdu().unwrap()
    }

    fn read_u32(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes[..4].try_into().unwrap())
    }

    fn response_status(encoded: &[u8]) -> NtStatus {
        NtStatus::from(read_u32(&encoded[12..]))
    }

    #[test]
    fn create_write_close_delivers_completed_print_job() {
        let (mut backend, mut rx) = printer_backend();

        let create_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(1)))
                .unwrap(),
        );
        assert_eq!(response_status(&create_response), NtStatus::SUCCESS);
        let file_id = read_u32(&create_response[16..]);

        let write_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(file_id, 2, b"hello".to_vec())))
                .unwrap(),
        );
        assert_eq!(response_status(&write_response), NtStatus::SUCCESS);
        assert_eq!(read_u32(&write_response[16..]), 5);

        let close_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(file_id, 3)))
                .unwrap(),
        );
        assert_eq!(response_status(&close_response), NtStatus::SUCCESS);

        match rx.try_recv().unwrap() {
            RdpInputEvent::Printer(PrinterBackendMessage::JobComplete {
                file_id: delivered_file_id,
                document_bytes,
                _queued_bytes: _,
            }) => {
                assert_eq!(delivered_file_id, file_id);
                assert_eq!(document_bytes, b"hello");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn write_for_unknown_file_id_is_rejected() {
        let (mut backend, _rx) = printer_backend();

        let response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(99, 1, b"lost".to_vec())))
                .unwrap(),
        );

        assert_eq!(response_status(&response), NtStatus::UNSUCCESSFUL);
        assert_eq!(read_u32(&response[16..]), 0);
    }

    #[test]
    fn oversized_write_rejects_and_drops_partial_job() {
        let (mut backend, _rx) = printer_backend_with_limits(4, MAX_QUEUED_PRINT_JOB_BYTES);

        let create_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(1)))
                .unwrap(),
        );
        let file_id = read_u32(&create_response[16..]);

        let write_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(
                    file_id,
                    2,
                    b"too large".to_vec(),
                )))
                .unwrap(),
        );
        assert_eq!(response_status(&write_response), NtStatus::UNSUCCESSFUL);
        assert_eq!(read_u32(&write_response[16..]), 0);

        let close_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(file_id, 3)))
                .unwrap(),
        );
        assert_eq!(response_status(&close_response), NtStatus::UNSUCCESSFUL);
    }

    #[test]
    fn completed_job_queue_budget_rejects_second_pending_job() {
        let (mut backend, mut rx) = printer_backend_with_limits(MAX_PRINT_JOB_BYTES, 4);

        let first_create = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(1)))
                .unwrap(),
        );
        let first_file_id = read_u32(&first_create[16..]);
        response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(
                    first_file_id,
                    2,
                    b"1234".to_vec(),
                )))
                .unwrap(),
        );
        let first_close = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(first_file_id, 3)))
                .unwrap(),
        );
        assert_eq!(response_status(&first_close), NtStatus::SUCCESS);

        let second_create = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(4)))
                .unwrap(),
        );
        let second_file_id = read_u32(&second_create[16..]);
        response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(second_file_id, 5, b"1".to_vec())))
                .unwrap(),
        );
        let second_close = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(second_file_id, 6)))
                .unwrap(),
        );
        assert_eq!(response_status(&second_close), NtStatus::UNSUCCESSFUL);

        match rx.try_recv().unwrap() {
            RdpInputEvent::Printer(PrinterBackendMessage::JobComplete { document_bytes, .. }) => {
                assert_eq!(document_bytes, b"1234");
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }
}
