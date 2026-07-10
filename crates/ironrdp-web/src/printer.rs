//! Browser-side virtual printer backend for RDPDR.
//!
//! Architecture mirrors the clipboard backend ([`crate::clipboard`]):
//!
//! * [`WasmPrinterBackend`] lives on the SVC processor side and implements
//!   [`ironrdp::rdpdr::backend::RdpdrBackend`]. It is `Send` (required by the
//!   trait) and holds only per-handle byte counts plus an mpsc proxy — no
//!   JS callbacks.
//! * [`WasmPrinter`] lives in the session event loop and owns the
//!   `js_sys::Function` callbacks. Per-job stream messages flow from the
//!   backend to the event loop via [`PrinterBackendMessage`].
//!
//! The IRP completion responses (DR_CREATE_RSP / DR_WRITE_RSP / DR_CLOSE_RSP)
//! are synthesised synchronously inside the backend — the RDP peer tracks
//! outstanding IRPs by `completion_id`, so completions just need to get
//! queued onto the SVC out-stream; they don't need JS roundtrips. Print data
//! is streamed to the event loop as writes arrive, so completed jobs are not
//! buffered inside the RDPDR backend. Each
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
/// Maximum pending print data bytes allowed to wait in the event queue.
const MAX_QUEUED_PRINT_DATA_BYTES: usize = MAX_PRINT_JOB_BYTES;

/// Messages sent from the printer backend to the session event loop.
#[derive(Debug)]
pub(crate) enum PrinterBackendMessage {
    /// A server-created printer file handle started a new print job.
    Created { file_id: u32 },
    /// A print job data chunk produced by the announced server-side driver.
    Data {
        file_id: u32,
        document_bytes: Vec<u8>,
        _queued_bytes: QueuedPrintDataBytes,
    },
    /// The server closed the print job file handle.
    Completed { file_id: u32 },
    /// The backend rejected or dropped the job before completion.
    Aborted { file_id: u32 },
}

pub(crate) struct QueuedPrintDataBytes {
    len: usize,
    queued_bytes: Arc<AtomicUsize>,
}

impl Drop for QueuedPrintDataBytes {
    fn drop(&mut self) {
        self.queued_bytes.fetch_sub(self.len, Ordering::AcqRel);
    }
}

impl fmt::Debug for QueuedPrintDataBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueuedPrintDataBytes")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// mpsc proxy used by the backend to stream print jobs to the event loop.
#[derive(Debug, Clone)]
pub(crate) struct WasmPrinterMessageProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
    queued_data_bytes: Arc<AtomicUsize>,
    queued_data_bytes_limit: usize,
}

impl WasmPrinterMessageProxy {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self::new_with_limit(tx, MAX_QUEUED_PRINT_DATA_BYTES)
    }

    fn send_job_created(&self, file_id: u32) -> bool {
        self.send_message(PrinterBackendMessage::Created { file_id })
    }

    fn send_job_data(&self, file_id: u32, document_bytes: Vec<u8>) -> bool {
        let Some(queued_bytes) = self.reserve_queue_capacity(document_bytes.len()) else {
            warn!(
                file_id,
                bytes = document_bytes.len(),
                limit = self.queued_data_bytes_limit,
                "Print job data exceeds queued print data byte budget"
            );
            return false;
        };

        self.send_message(PrinterBackendMessage::Data {
            file_id,
            document_bytes,
            _queued_bytes: queued_bytes,
        })
    }

    fn send_job_completed(&self, file_id: u32) -> bool {
        self.send_message(PrinterBackendMessage::Completed { file_id })
    }

    fn send_job_aborted(&self, file_id: u32) {
        let _ = self.send_message(PrinterBackendMessage::Aborted { file_id });
    }

    fn send_message(&self, message: PrinterBackendMessage) -> bool {
        if self.tx.unbounded_send(RdpInputEvent::Printer(message)).is_err() {
            error!("Failed to queue printer backend message, event loop receiver is closed");
            return false;
        }

        true
    }

    fn reserve_queue_capacity(&self, len: usize) -> Option<QueuedPrintDataBytes> {
        let mut queued = self.queued_data_bytes.load(Ordering::Acquire);
        loop {
            let next = queued.checked_add(len)?;
            if next > self.queued_data_bytes_limit {
                return None;
            }

            match self
                .queued_data_bytes
                .compare_exchange_weak(queued, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    return Some(QueuedPrintDataBytes {
                        len,
                        queued_bytes: Arc::clone(&self.queued_data_bytes),
                    });
                }
                Err(actual) => queued = actual,
            }
        }
    }

    fn new_with_limit(tx: mpsc::UnboundedSender<RdpInputEvent>, queued_data_bytes_limit: usize) -> Self {
        Self {
            tx,
            queued_data_bytes: Arc::new(AtomicUsize::new(0)),
            queued_data_bytes_limit,
        }
    }
}

#[derive(Debug)]
struct OpenPrintJob {
    bytes_written: usize,
}

/// RDPDR backend that streams a server-initiated print job to the session
/// event loop as write IRPs arrive.
#[derive(Debug)]
pub(crate) struct WasmPrinterBackend {
    /// Per-file-handle document byte counts. Populated on `IRP_MJ_CREATE`,
    /// updated by `IRP_MJ_WRITE`, drained on `IRP_MJ_CLOSE`.
    open_files: HashMap<u32, OpenPrintJob>,
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
                let io_status = if self.proxy.send_job_created(file_id) {
                    self.open_files.insert(file_id, OpenPrintJob { bytes_written: 0 });
                    trace!(file_id, path = %create.path, "IRP_MJ_CREATE: opened print handle");
                    NtStatus::SUCCESS
                } else {
                    NtStatus::UNSUCCESSFUL
                };

                let response = DeviceCreateResponse {
                    device_io_reply: DeviceIoResponse::new(create.device_io_request, io_status),
                    file_id,
                    // A virtual printer is conceptually opened fresh every time;
                    // the bridge's former implementation used FILE_OPENED and
                    // Windows' own redirector accepts either value.
                    information: if io_status == NtStatus::SUCCESS {
                        Information::FILE_OPENED
                    } else {
                        Information::empty()
                    },
                };
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(response))])
            }
            PrinterIoRequest::Write(write) => {
                let file_id = write.device_io_request.file_id;
                let device_io_request = write.device_io_request;
                let write_data = write.write_data;
                // INVARIANT: write.write_data was decoded via a u32-length-prefixed
                // wire field (MS-RDPEFS 2.2.1.4.4 DR_WRITE_REQ Length), so its
                // in-memory Vec length always round-trips back to a u32.
                let data_len = u32::try_from(write_data.len()).expect("write length round-trips from u32 wire decode");

                let mut drop_partial_job = false;
                let io_status = match self.open_files.get_mut(&file_id) {
                    Some(job) => {
                        if let Some(projected_len) = job
                            .bytes_written
                            .checked_add(write_data.len())
                            .filter(|len| *len <= self.max_print_job_bytes)
                        {
                            if self.proxy.send_job_data(file_id, write_data) {
                                job.bytes_written = projected_len;
                                trace!(
                                    file_id,
                                    chunk = data_len,
                                    total = job.bytes_written,
                                    "IRP_MJ_WRITE: streamed"
                                );
                                NtStatus::SUCCESS
                            } else {
                                warn!(
                                    file_id,
                                    chunk = data_len,
                                    current = job.bytes_written,
                                    limit = self.max_print_job_bytes,
                                    "IRP_MJ_WRITE could not be queued; rejecting and dropping partial job"
                                );
                                drop_partial_job = true;
                                NtStatus::UNSUCCESSFUL
                            }
                        } else {
                            warn!(
                                file_id,
                                chunk = data_len,
                                current = job.bytes_written,
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
                    self.proxy.send_job_aborted(file_id);
                }

                let response = DeviceWriteResponse {
                    device_io_reply: DeviceIoResponse::new(device_io_request, io_status),
                    length: if io_status == NtStatus::SUCCESS { data_len } else { 0 },
                };
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(response))])
            }
            PrinterIoRequest::Close(close) => {
                let file_id = close.device_io_request.file_id;
                let io_status = if let Some(job) = self.open_files.remove(&file_id) {
                    debug!(
                        file_id,
                        bytes = job.bytes_written,
                        "IRP_MJ_CLOSE: completing streamed print job"
                    );
                    if self.proxy.send_job_completed(file_id) {
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
    callbacks: JsPrinterStreamCallbacks,
}

#[derive(Debug, Clone)]
pub(crate) struct JsPrinterStreamCallbacks {
    /// Optional `function(fileId: number): void`.
    pub(crate) on_job_start: Option<js_sys::Function>,
    /// Required `function(fileId: number, chunk: Uint8Array): void`.
    pub(crate) on_job_data: js_sys::Function,
    /// Required `function(fileId: number): void`.
    pub(crate) on_job_complete: js_sys::Function,
    /// Optional `function(fileId: number): void`.
    pub(crate) on_job_error: Option<js_sys::Function>,
}

impl WasmPrinter {
    pub(crate) fn new(callbacks: JsPrinterStreamCallbacks) -> Self {
        Self { callbacks }
    }

    pub(crate) fn process_message(&self, message: PrinterBackendMessage) {
        let this = JsValue::NULL;
        match message {
            PrinterBackendMessage::Created { file_id } => {
                if let Some(on_job_start) = &self.callbacks.on_job_start {
                    let file_id = JsValue::from(file_id);
                    if let Err(err) = on_job_start.call1(&this, &file_id) {
                        error!(?err, "on_job_start JS callback threw");
                    }
                }
            }
            PrinterBackendMessage::Data {
                file_id,
                document_bytes,
                _queued_bytes: _,
            } => {
                trace!(
                    file_id,
                    bytes = document_bytes.len(),
                    "Delivering print data chunk to JS callback"
                );
                let file_id = JsValue::from(file_id);
                let array = js_sys::Uint8Array::from(document_bytes.as_slice());
                if let Err(err) = self.callbacks.on_job_data.call2(&this, &file_id, &array) {
                    error!(?err, "on_job_data JS callback threw");
                }
            }
            PrinterBackendMessage::Completed { file_id } => {
                let file_id = JsValue::from(file_id);
                if let Err(err) = self.callbacks.on_job_complete.call1(&this, &file_id) {
                    error!(?err, "on_job_complete JS callback threw");
                }
            }
            PrinterBackendMessage::Aborted { file_id } => {
                if let Some(on_job_error) = &self.callbacks.on_job_error {
                    let file_id = JsValue::from(file_id);
                    if let Err(err) = on_job_error.call1(&this, &file_id) {
                        error!(?err, "on_job_error JS callback threw");
                    }
                }
            }
        }
    }
}

/// Factory used by [`crate::session::SessionBuilder::connect`] to build a
/// matched (backend, event-loop) pair from a single mpsc channel.
pub(crate) fn wasm_printer_pair(
    input_events_tx: mpsc::UnboundedSender<RdpInputEvent>,
    callbacks: JsPrinterStreamCallbacks,
) -> (WasmPrinterBackend, WasmPrinter) {
    let proxy = WasmPrinterMessageProxy::new(input_events_tx);
    let backend = WasmPrinterBackend::new(proxy);
    let printer = WasmPrinter::new(callbacks);
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
        printer_backend_with_limits(MAX_PRINT_JOB_BYTES, MAX_QUEUED_PRINT_DATA_BYTES)
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

    fn expect_job_created(rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>, expected_file_id: u32) {
        match rx.try_recv().unwrap() {
            RdpInputEvent::Printer(PrinterBackendMessage::Created { file_id }) => {
                assert_eq!(file_id, expected_file_id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    fn expect_job_data(rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>, expected_file_id: u32, expected_data: &[u8]) {
        match rx.try_recv().unwrap() {
            RdpInputEvent::Printer(PrinterBackendMessage::Data {
                file_id,
                document_bytes,
                _queued_bytes: _,
            }) => {
                assert_eq!(file_id, expected_file_id);
                assert_eq!(document_bytes, expected_data);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    fn expect_job_completed(rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>, expected_file_id: u32) {
        match rx.try_recv().unwrap() {
            RdpInputEvent::Printer(PrinterBackendMessage::Completed { file_id }) => {
                assert_eq!(file_id, expected_file_id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    fn expect_job_aborted(rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>, expected_file_id: u32) {
        match rx.try_recv().unwrap() {
            RdpInputEvent::Printer(PrinterBackendMessage::Aborted { file_id }) => {
                assert_eq!(file_id, expected_file_id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn create_write_close_streams_print_job_events() {
        let (mut backend, mut rx) = printer_backend();

        let create_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(1)))
                .unwrap(),
        );
        assert_eq!(response_status(&create_response), NtStatus::SUCCESS);
        let file_id = read_u32(&create_response[16..]);
        expect_job_created(&mut rx, file_id);

        let write_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(file_id, 2, b"hello".to_vec())))
                .unwrap(),
        );
        assert_eq!(response_status(&write_response), NtStatus::SUCCESS);
        assert_eq!(read_u32(&write_response[16..]), 5);
        expect_job_data(&mut rx, file_id, b"hello");

        let close_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(file_id, 3)))
                .unwrap(),
        );
        assert_eq!(response_status(&close_response), NtStatus::SUCCESS);
        expect_job_completed(&mut rx, file_id);
        assert!(rx.try_recv().is_err());
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
        let (mut backend, mut rx) = printer_backend_with_limits(4, MAX_QUEUED_PRINT_DATA_BYTES);

        let create_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(1)))
                .unwrap(),
        );
        let file_id = read_u32(&create_response[16..]);
        expect_job_created(&mut rx, file_id);

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
        expect_job_aborted(&mut rx, file_id);

        let close_response = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(file_id, 3)))
                .unwrap(),
        );
        assert_eq!(response_status(&close_response), NtStatus::UNSUCCESSFUL);
    }

    #[test]
    fn queued_print_data_budget_rejects_second_pending_chunk() {
        let (mut backend, mut rx) = printer_backend_with_limits(MAX_PRINT_JOB_BYTES, 4);

        let first_create = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(1)))
                .unwrap(),
        );
        let first_file_id = read_u32(&first_create[16..]);
        expect_job_created(&mut rx, first_file_id);
        response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(
                    first_file_id,
                    2,
                    b"1234".to_vec(),
                )))
                .unwrap(),
        );

        let second_create = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Create(create_request(4)))
                .unwrap(),
        );
        let second_file_id = read_u32(&second_create[16..]);
        let second_write = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Write(write_request(second_file_id, 5, b"1".to_vec())))
                .unwrap(),
        );
        assert_eq!(response_status(&second_write), NtStatus::UNSUCCESSFUL);
        assert_eq!(read_u32(&second_write[16..]), 0);

        expect_job_data(&mut rx, first_file_id, b"1234");
        expect_job_created(&mut rx, second_file_id);
        expect_job_aborted(&mut rx, second_file_id);

        let first_close = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(first_file_id, 3)))
                .unwrap(),
        );
        assert_eq!(response_status(&first_close), NtStatus::SUCCESS);
        expect_job_completed(&mut rx, first_file_id);

        let second_close = response_bytes(
            backend
                .handle_printer_io_request(PrinterIoRequest::Close(close_request(second_file_id, 6)))
                .unwrap(),
        );
        assert_eq!(response_status(&second_close), NtStatus::UNSUCCESSFUL);
        assert!(rx.try_recv().is_err());
    }
}
