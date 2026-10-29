//! A device claimed through the RemoteFX USB redirection drivers, served on its device channel.
//!
//! The generic redirection driver (TsUsbGD) takes each `TS_URB` as the server sent it, and writes
//! the `TS_URB_RESULT` back over it, so requests are re-encoded rather than translated. Isochronous
//! transfers and the cancellation of a single request are supported.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ironrdp_pdu::PduResult;
use ironrdp_rdpeusb::client::UrbdrcDeviceBackend;
use ironrdp_rdpeusb::io::device::DeviceInfo;
use ironrdp_rdpeusb::io::{
    CompletionData, DeviceText, InternalIoControlPacket, IoControlCompletionResult, IoControlPacket, IoctlInternalUsb,
    RequestId, TransferInCompletionResult, TransferInPacket, TransferOutCompletionResult, TransferOutPacket,
    TsUrbInKind,
};
use ironrdp_rdpeusb::pdu::completion::ts_urb_result::{TsUrbIsochTransferResult, TsUrbResultPayload};
use ironrdp_rdpeusb::pdu::utils::UsbdStatus;
use tracing::{debug, warn};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, ERROR_OPERATION_ABORTED};
use windows::core::{Error, GUID};

use super::control::{braced_guid, utf16_string};
use super::io::{AsyncDevice, DeviceHandle, IoResult};
use crate::device::{OnClose, Reply, TransferBudget};
use crate::manager::{DeviceKey, RedirectableDevice};
use crate::urb::{self, usbd_status};

/// Submit a `TS_URB`. The input buffer also receives the `TS_URB_RESULT`.
const IOCTL_SUBMIT_URB: u32 = 0x0022_c40b;
/// Cancel one request, by the handle it was submitted on and the address of its `OVERLAPPED`.
const IOCTL_CANCEL_REQUEST: u32 = 0x0022_c400;
/// Query the proprietary ID the device was claimed by.
const IOCTL_QUERY_PROPRIETARY_ID: u32 = 0x0022_4434;
/// Query a device text, by `{u32 text type, u32 locale}`, as `{u16 length, text}`.
const IOCTL_QUERY_DEVICE_TEXT: u32 = 0x0022_440c;
const IOCTL_QUERY_BUS_TIME: u32 = 0x0022_4000;
const IOCTL_RESET_PORT: u32 = 0x0022_8418;
const IOCTL_GET_PORT_STATUS: u32 = 0x0022_4410;
const IOCTL_GET_HUB_COUNT: u32 = 0x0022_441c;
const IOCTL_CYCLE_PORT: u32 = 0x0022_4420;
const IOCTL_GET_HUB_NAME: u32 = 0x0022_4424;
const IOCTL_GET_BUS_INFO: u32 = 0x0022_4428;
const IOCTL_GET_CONTROLLER_NAME: u32 = 0x0022_442c;

/// Room for the result of a request that selects a configuration or an interface, which grows with
/// the pipes of the selected interfaces.
const SELECT_RESULT_BUFFER: usize = 0xffff;
/// Device text buffer, enough for most descriptions; longer ones are queried again.
const DEVICE_TEXT_BUFFER: usize = 2 + 2 * 512;
const MAX_DEVICE_TEXT_BUFFER: usize = 2 + 0xffff;

/// The handles of a claimed device, before it is matched with what it was claimed as.
pub(crate) struct ClaimedDevice {
    pub(crate) proprietary_id: String,
    sync: DeviceHandle,
    io: AsyncDevice,
}

impl ClaimedDevice {
    /// Open the device behind a TsUsbGD interface. Only the process that claimed it can.
    pub(crate) fn open(interface_path: &str, claim_guid: &GUID) -> Result<Self, Error> {
        let path = format!(r"{interface_path}\{}", braced_guid(claim_guid));
        let sync = DeviceHandle::open_read_write(&path, false)?;
        let proprietary_id = utf16_string(&sync.control_vec(IOCTL_QUERY_PROPRIETARY_ID, &[])?);
        let io = AsyncDevice::new(DeviceHandle::open_read_write(&path, true)?, "urbdrc-tsusbgd")?;
        Ok(Self {
            proprietary_id,
            sync,
            io,
        })
    }

    /// Attach what the device was described as before it was claimed.
    ///
    /// `release` runs once the device is no longer redirected, or if it never is, to give it back to
    /// its own driver.
    pub(crate) fn describe(
        self,
        info: DeviceInfo,
        description: String,
        release: Box<dyn FnOnce() + Send>,
    ) -> TsUsbDevice {
        TsUsbDevice {
            claimed: self,
            info,
            description,
            release: OnClose(Some(release)),
        }
    }
}

/// A claimed device, ready to be redirected.
pub(crate) struct TsUsbDevice {
    claimed: ClaimedDevice,
    info: DeviceInfo,
    description: String,
    release: OnClose,
}

impl RedirectableDevice for TsUsbDevice {
    fn key(&self) -> DeviceKey {
        DeviceKey::TsUsb(self.claimed.proprietary_id.clone())
    }

    fn vendor_product(&self) -> (u16, u16) {
        (self.info.descriptor.vendor_id, self.info.descriptor.product_id)
    }

    fn start(
        self: Box<Self>,
        reply: Reply,
        events_token: Arc<()>,
        on_close: Box<dyn FnOnce() + Send>,
    ) -> Box<dyn UrbdrcDeviceBackend> {
        let Self {
            claimed: ClaimedDevice { sync, io, .. },
            info,
            description,
            release,
        } = *self;
        Box::new(TsUsbDeviceBackend {
            info,
            description,
            sync,
            io,
            requests: Arc::new(Mutex::new(HashMap::new())),
            budget: TransferBudget::new(),
            reply,
            _events_token: events_token,
            _on_close: OnClose(Some(on_close)),
            _release: release,
        })
    }
}

type Requests = Mutex<HashMap<RequestId, u64>>;

fn requests(requests: &Requests) -> MutexGuard<'_, HashMap<RequestId, u64>> {
    // Every update is a single map operation, so the map stays consistent across a panic.
    requests.lock().unwrap_or_else(PoisonError::into_inner)
}

/// URBDRC device backend for a device claimed through the RemoteFX USB redirection drivers.
pub(crate) struct TsUsbDeviceBackend {
    info: DeviceInfo,
    description: String,
    /// Synchronous handle, for queries and cancellation.
    sync: DeviceHandle,
    /// Overlapped handle, for requests. Dropping it cancels them and waits for them to return.
    io: AsyncDevice,
    /// Requests in flight, by request ID, with their key on `io`.
    requests: Arc<Requests>,
    budget: Arc<TransferBudget>,
    reply: Reply,
    /// Keeps the events of this device current; they are dropped once the device channel is gone.
    _events_token: Arc<()>,
    /// Dropped once every request returned.
    _on_close: OnClose,
    /// Dropped last: gives the device back to its own driver.
    _release: OnClose,
}

impl TsUsbDeviceBackend {
    /// Submit a request, returning `false` when the device already holds too much data in flight.
    ///
    /// `complete` turns the driver's result into the completion to send, if any. A request that is
    /// not `cancellable` is not tracked for cancellation: the server never cancels a request without
    /// a completion, and may reuse its ID while it runs.
    fn submit(
        &self,
        request_id: RequestId,
        code: u32,
        input: Vec<u8>,
        output: Vec<u8>,
        cancellable: bool,
        complete: impl FnOnce(IoResult) -> Option<CompletionData> + Send + 'static,
    ) -> bool {
        let bytes = input
            .len()
            .saturating_add(output.len())
            .saturating_add(urb::REQUEST_OVERHEAD_BYTES);
        let Some(reservation) = self.budget.reserve(bytes) else {
            return false;
        };

        let key = self.io.next_key();
        if cancellable {
            requests(&self.requests).insert(request_id, key);
        }
        let in_flight = Arc::clone(&self.requests);
        let reply = self.reply.clone();
        self.io.submit(key, code, input, output, move |result| {
            let _reservation = reservation;
            {
                // Only this request's own entry: a later request may have reused its ID.
                let mut in_flight = requests(&in_flight);
                if in_flight.get(&request_id) == Some(&key) {
                    in_flight.remove(&request_id);
                }
            }
            if let Some(completion) = complete(result) {
                reply.complete(request_id, completion);
            }
        });
        true
    }

    fn device_text(&self, input: &[u8]) -> Result<String, Error> {
        let mut output = vec![0; DEVICE_TEXT_BUFFER];
        let length = match self.sync.control(IOCTL_QUERY_DEVICE_TEXT, input, &mut output) {
            Err(error)
                if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult()
                    || error.code() == ERROR_MORE_DATA.to_hresult() =>
            {
                output.resize(MAX_DEVICE_TEXT_BUFFER, 0);
                self.sync.control(IOCTL_QUERY_DEVICE_TEXT, input, &mut output)?
            }
            result => result?,
        };
        output.truncate(length);
        // The text follows its length in characters.
        Ok(utf16_string(output.get(2..).unwrap_or_default()))
    }
}

impl UrbdrcDeviceBackend for TsUsbDeviceBackend {
    fn device_info(&mut self, _channel_id: u32) -> PduResult<DeviceInfo> {
        Ok(self.info.clone())
    }

    fn cancel_request(&mut self, request_id: RequestId, _channel_id: u32) {
        let Some(key) = requests(&self.requests).get(&request_id).copied() else {
            return;
        };
        let sync = &self.sync;
        self.io.cancel_with(key, |handle, overlapped| {
            let mut input = Vec::with_capacity(16);
            input.extend_from_slice(&u64::try_from(handle.0.addr()).unwrap_or(0).to_le_bytes());
            input.extend_from_slice(&u64::try_from(overlapped).unwrap_or(0).to_le_bytes());
            sync.control(IOCTL_CANCEL_REQUEST, &input, &mut []).is_ok()
        });
    }

    fn query_device_text(&mut self, _channel_id: u32, text_type: u32, locale_id: u32) -> PduResult<DeviceText> {
        let mut input = text_type.to_le_bytes().to_vec();
        input.extend_from_slice(&locale_id.to_le_bytes());
        Ok(match self.device_text(&input) {
            Ok(description) => DeviceText {
                hresult: urb::S_OK,
                description,
            },
            Err(error) => {
                debug!(%error, text_type, "USB device text query failed");
                urb::device_text(text_type, &self.description, "")
            }
        })
    }

    fn io_control(
        &mut self,
        _channel_id: u32,
        request_id: RequestId,
        request: IoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        let code = match request.ioctl_code {
            IoctlInternalUsb::ResetPort => IOCTL_RESET_PORT,
            IoctlInternalUsb::GetPortStatus => IOCTL_GET_PORT_STATUS,
            IoctlInternalUsb::GetHubCount => IOCTL_GET_HUB_COUNT,
            IoctlInternalUsb::CyclePort => IOCTL_CYCLE_PORT,
            IoctlInternalUsb::GetHubName => IOCTL_GET_HUB_NAME,
            IoctlInternalUsb::GetBusInfo => IOCTL_GET_BUS_INFO,
            IoctlInternalUsb::GetControllerName => IOCTL_GET_CONTROLLER_NAME,
            _ => return Ok(Some(urb::io_control(&request))),
        };
        let output = vec![0; usize::try_from(request.output_buffer_size).unwrap_or(0)];
        let submitted = self.submit(request_id, code, request.input_buffer, output, true, |result| {
            Some(CompletionData::IoControl(io_control_completion(result)))
        });
        Ok((!submitted).then(|| io_control_failure(urb::HRESULT_GEN_FAILURE)))
    }

    fn internal_io_control(
        &mut self,
        _channel_id: u32,
        request_id: RequestId,
        request: InternalIoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        let InternalIoControlPacket::QueryBusTime = request;
        let submitted = self.submit(
            request_id,
            IOCTL_QUERY_BUS_TIME,
            Vec::new(),
            vec![0; 4],
            true,
            |result| Some(CompletionData::InternalIoControl(io_control_completion(result))),
        );
        Ok((!submitted).then(|| io_control_failure(urb::HRESULT_GEN_FAILURE)))
    }

    fn transfer_in(
        &mut self,
        _channel_id: u32,
        request_id: RequestId,
        request: TransferInPacket,
    ) -> PduResult<Option<TransferInCompletionResult>> {
        if urb::MAX_TRANSFER_LENGTH < request.output_buffer_size {
            return Ok(Some(transfer_in_failure(usbd_status::INVALID_PARAMETER)));
        }

        let mut input = request.ts_urb.encode_ts_urb(request_id)?;
        let result_room = if matches!(
            request.ts_urb.kind,
            TsUrbInKind::SelectConfig(_) | TsUrbInKind::SelectIface(_)
        ) {
            SELECT_RESULT_BUFFER
        } else {
            input.len().saturating_mul(2)
        };
        input.resize(result_room.max(input.len()), 0);
        let output = vec![0; usize::try_from(request.output_buffer_size).unwrap_or(0)];

        let ts_urb = request.ts_urb;
        let submitted = self.submit(request_id, IOCTL_SUBMIT_URB, input, output, true, move |result| {
            let IoResult {
                status,
                input,
                mut output,
                transferred,
            } = result;
            let mut ts_urb_result = match status {
                Err(error) => {
                    return Some(CompletionData::TransferIn(transfer_in_failure(driver_status(
                        &input, &error,
                    ))));
                }
                Ok(()) => match ts_urb.decode_ts_urb_result(&input) {
                    Ok(ts_urb_result) => ts_urb_result,
                    Err(error) => {
                        warn!(%error, "USB redirection driver returned an invalid TS_URB_RESULT");
                        return Some(CompletionData::TransferIn(transfer_in_failure(
                            usbd_status::REQUEST_FAILED,
                        )));
                    }
                },
            };
            let output_buffer = match &mut ts_urb_result.payload {
                Some(TsUrbResultPayload::Isoch(isoch)) => isochronous_data(&output, isoch),
                _ => {
                    output.truncate(transferred);
                    output
                }
            };
            let hresult = urb::hresult(ts_urb_result.header.usbd_status);
            Some(CompletionData::TransferIn(TransferInCompletionResult {
                ts_urb_result,
                hresult,
                output_buffer,
            }))
        });
        Ok((!submitted).then(|| transfer_in_failure(usbd_status::INSUFFICIENT_RESOURCES)))
    }

    fn transfer_out(
        &mut self,
        _channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    ) -> PduResult<Option<TransferOutCompletionResult>> {
        let mut input = request.ts_urb.encode_ts_urb(request_id)?;
        input.resize(input.len().saturating_mul(2), 0);

        let ts_urb = request.ts_urb;
        let submitted = self.submit(
            request_id,
            IOCTL_SUBMIT_URB,
            input,
            request.output_buffer,
            true,
            move |result| {
                let IoResult {
                    status,
                    input,
                    transferred,
                    ..
                } = result;
                let completion = match status.map(|()| ts_urb.decode_ts_urb_result(&input)) {
                    Err(error) => transfer_out_failure(driver_status(&input, &error)),
                    Ok(Err(error)) => {
                        warn!(%error, "USB redirection driver returned an invalid TS_URB_RESULT");
                        transfer_out_failure(usbd_status::REQUEST_FAILED)
                    }
                    Ok(Ok(ts_urb_result)) => TransferOutCompletionResult {
                        hresult: urb::hresult(ts_urb_result.header.usbd_status),
                        ts_urb_result,
                        output_buffer_size: u32::try_from(transferred).unwrap_or(u32::MAX),
                    },
                };
                Some(CompletionData::TransferOut(completion))
            },
        );
        Ok((!submitted).then(|| transfer_out_failure(usbd_status::INSUFFICIENT_RESOURCES)))
    }

    fn transfer_out_no_ack(
        &mut self,
        _channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    ) -> PduResult<()> {
        let mut input = request.ts_urb.encode_ts_urb(request_id)?;
        input.resize(input.len().saturating_mul(2), 0);
        let submitted = self.submit(
            request_id,
            IOCTL_SUBMIT_URB,
            input,
            request.output_buffer,
            false,
            |result| {
                if let Err(error) = result.status {
                    debug!(%error, "USB isochronous write without acknowledgement failed");
                }
                None
            },
        );
        if !submitted {
            debug!("Dropping a USB isochronous write: too much data in flight");
        }
        Ok(())
    }

    fn retract(&mut self, _channel_id: u32) -> PduResult<()> {
        self.reply.terminate();
        Ok(())
    }
}

/// The USBD status of a failed request.
///
/// The driver writes a header-only `TS_URB_RESULT` with the URB status when the USB stack fails a
/// request. A request it rejected before submitting it still holds the `TS_URB`, whose request ID is
/// never an error status.
fn driver_status(input: &[u8], error: &Error) -> UsbdStatus {
    let size = input
        .get(..2)
        .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
        .map(u16::from_le_bytes);
    let status = input
        .get(4..8)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_le_bytes);
    match (size, status) {
        (Some(8), Some(status)) if usbd_status::is_error(status) => status,
        _ if error.code() == ERROR_OPERATION_ABORTED.to_hresult() => usbd_status::CANCELED,
        _ => usbd_status::REQUEST_FAILED,
    }
}

/// The data of an isochronous IN transfer: the bytes of every packet received, in packet order.
///
/// The server reads each successful packet's length from the data in turn, so a packet whose bytes
/// are not in the buffer is reported as failed instead of being skipped.
fn isochronous_data(buffer: &[u8], result: &mut TsUrbIsochTransferResult) -> Vec<u8> {
    let mut data = Vec::new();
    for packet in result
        .iso_packet
        .iter_mut()
        .filter(|packet| 0 <= packet.status && packet.length != 0)
    {
        let range = usize::try_from(packet.offset)
            .ok()
            .zip(usize::try_from(packet.length).ok())
            .and_then(|(offset, length)| Some(offset..offset.checked_add(length)?));
        match range.and_then(|range| buffer.get(range)) {
            Some(bytes) => data.extend_from_slice(bytes),
            None => {
                packet.status = i32::from_ne_bytes(usbd_status::REQUEST_FAILED.to_ne_bytes());
                packet.length = 0;
                result.error_count = result.error_count.saturating_add(1);
            }
        }
    }
    data
}

fn transfer_in_failure(status: UsbdStatus) -> TransferInCompletionResult {
    TransferInCompletionResult {
        ts_urb_result: urb::result(status, None),
        hresult: urb::hresult(status),
        output_buffer: Vec::new(),
    }
}

fn transfer_out_failure(status: UsbdStatus) -> TransferOutCompletionResult {
    TransferOutCompletionResult {
        ts_urb_result: urb::result(status, None),
        hresult: urb::hresult(status),
        output_buffer_size: 0,
    }
}

fn io_control_completion(result: IoResult) -> IoControlCompletionResult {
    match result.status {
        Ok(()) => {
            let mut output = result.output;
            output.truncate(result.transferred);
            IoControlCompletionResult {
                hresult: urb::S_OK,
                information: u32::try_from(output.len()).unwrap_or(u32::MAX),
                output_buffer: output,
            }
        }
        Err(error) => io_control_failure(u32::from_ne_bytes(error.code().0.to_ne_bytes())),
    }
}

fn io_control_failure(hresult: u32) -> IoControlCompletionResult {
    IoControlCompletionResult {
        hresult,
        information: 0,
        output_buffer: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_rdpeusb::pdu::utils::UsbdIsoPacketDesc;

    use super::*;

    fn packet(offset: u32, length: u32, status: i32) -> UsbdIsoPacketDesc {
        UsbdIsoPacketDesc { offset, length, status }
    }

    #[test]
    fn isochronous_data_keeps_received_packets_in_order() {
        let buffer = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut result = TsUrbIsochTransferResult {
            start_frame: 0,
            error_count: 1,
            iso_packet: vec![
                packet(0, 2, 0),
                packet(2, 2, -1),
                packet(4, 0, 0),
                packet(6, 2, 0),
                packet(7, 4, 0),
            ],
        };
        assert_eq!(isochronous_data(&buffer, &mut result), [1, 2, 7, 8]);

        // The packet past the end of the buffer is reported as failed, so the data still matches the
        // successful packets' lengths.
        let last = result.iso_packet.last().expect("five packets");
        assert!(last.status < 0);
        assert_eq!(last.length, 0);
        assert_eq!(result.error_count, 2);
        let successful: u32 = result
            .iso_packet
            .iter()
            .filter(|packet| 0 <= packet.status)
            .map(|packet| packet.length)
            .sum();
        assert_eq!(successful, 4);
    }

    #[test]
    fn failures_report_the_driver_status_when_it_wrote_one() {
        let aborted = Error::from(ERROR_OPERATION_ABORTED);
        let stalled = [8, 0, 0, 0, 0x04, 0, 0, 0xc0];
        assert_eq!(driver_status(&stalled, &aborted), usbd_status::STALL_PID);

        // A rejected request still holds its TS_URB, with a request ID at the same offset.
        let rejected = [16, 0, 9, 0, 7, 0, 0, 0];
        assert_eq!(driver_status(&rejected, &aborted), usbd_status::CANCELED);
        assert_eq!(
            driver_status(&rejected, &Error::from(ERROR_MORE_DATA)),
            usbd_status::REQUEST_FAILED
        );
    }
}
