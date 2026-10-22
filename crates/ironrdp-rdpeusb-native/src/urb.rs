//! Translation between RDPEUSB `TS_URB` requests and host USB operations.
//!
//! Everything here is sans-I/O so that it can be unit-tested without a device: the device worker
//! executes the returned [`UsbRequest`] and feeds the outcome back through the completion builders.
//!
//! The mapping follows the WDK URB semantics that MS-RDPEUSB carries unchanged, and the FreeRDP
//! URBDRC client where the specification leaves room (pipe handle layout, port status values).

use core::time::Duration;

use ironrdp_rdpeusb::io::{
    CompletionData, DeviceText, HResult, IoControlCompletionResult, IoControlPacket, IoctlInternalUsb,
    TransferInCompletionResult, TransferInPacket, TransferOutCompletionResult, TransferOutPacket, TsUrbInKind,
    TsUrbOutKind, TsUrbResult, UrbFunction,
};
use ironrdp_rdpeusb::pdu::completion::ts_urb_result::{
    TsUrbIsochTransferResult, TsUrbResultHeader, TsUrbResultPayload, TsUrbSelectConfigResult,
    TsUrbSelectInterfaceResult, TsUsbdInterfaceInfoResult, TsUsbdPipeInfoResult, UsbdPipeType,
};
use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::utils::{SetupPacket as RdpeusbSetupPacket, TsUsbdInterfaceInfo};
use ironrdp_rdpeusb::pdu::utils::{ConfigHandle, PipeHandle, UsbdIsoPacketDesc, UsbdStatus};
use ironrdp_usb::control::{Recipient, RequestKind, RequestType, SetupPacket, standard_feature, standard_request};
use ironrdp_usb::descriptor::{ConfigurationDescriptorSet, InterfaceDescriptor};
use ironrdp_usb::{Direction, TransferType};

/// Windows `USBD_STATUS` values carried in `TS_URB_RESULT_HEADER`.
pub(crate) mod usbd_status {
    use ironrdp_rdpeusb::pdu::utils::UsbdStatus;

    pub(crate) const SUCCESS: UsbdStatus = 0x0000_0000;
    pub(crate) const STALL_PID: UsbdStatus = 0xc000_0004;
    pub(crate) const DEV_NOT_RESPONDING: UsbdStatus = 0xc000_0005;
    pub(crate) const INVALID_PARAMETER: UsbdStatus = 0x8000_0300;
    pub(crate) const REQUEST_FAILED: UsbdStatus = 0x8000_0500;
    pub(crate) const INVALID_PIPE_HANDLE: UsbdStatus = 0x8000_0600;
    pub(crate) const NOT_SUPPORTED: UsbdStatus = 0xc000_0e00;
    pub(crate) const CANCELED: UsbdStatus = 0xc001_0000;
    pub(crate) const INSUFFICIENT_RESOURCES: UsbdStatus = 0xc000_1000;
    pub(crate) const DEVICE_GONE: UsbdStatus = 0xc000_7000;

    /// `USBD_ERROR`: the high bit marks every failure status.
    pub(crate) const fn is_error(status: UsbdStatus) -> bool {
        status & 0x8000_0000 != 0
    }
}

/// `S_OK`.
pub(crate) const S_OK: HResult = 0;
/// `HRESULT_FROM_WIN32(ERROR_GEN_FAILURE)`.
///
/// A failed URB must also fail the IRP on the server, otherwise class drivers such as usbstor never
/// run their stall recovery. This matches the FreeRDP URBDRC client.
pub(crate) const HRESULT_GEN_FAILURE: HResult = 0x8007_001f;
/// `HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED)`.
const HRESULT_NOT_SUPPORTED: HResult = 0x8007_0032;

// Handles are opaque to the server. Tag them so that a null handle, which a Windows URB uses for the
// default control pipe, can never collide with one of ours, and keep the endpoint address in the low
// byte of pipe handles, like FreeRDP does.
const CONFIG_HANDLE_TAG: ConfigHandle = 0x0001_0000;
const INTERFACE_HANDLE_TAG: u32 = 0x0002_0000;
const PIPE_HANDLE_TAG: PipeHandle = 0x0003_0000;

/// `USBD_PORT_ENABLED | USBD_PORT_CONNECTED`, returned for `IOCTL_INTERNAL_USB_GET_PORT_STATUS`.
const PORT_STATUS_ENABLED_CONNECTED: u32 = 0x0000_0003;

/// `USBD_TRANSFER_DIRECTION_IN` transfer flag.
const USBD_TRANSFER_DIRECTION_IN: u32 = 0x0000_0001;

/// `DeviceTextDescription` from the WDK `DEVICE_TEXT_TYPE` enumeration.
const DEVICE_TEXT_DESCRIPTION: u32 = 0;
/// `DeviceTextLocationInformation` from the WDK `DEVICE_TEXT_TYPE` enumeration.
const DEVICE_TEXT_LOCATION_INFORMATION: u32 = 1;

/// Microsoft OS string descriptor index, whose `bMS_VendorCode` field selects the vendor request that
/// returns Microsoft OS feature descriptors.
pub(crate) const MS_OS_STRING_DESCRIPTOR_INDEX: u8 = 0xee;
/// Offset of `bMS_VendorCode` in the Microsoft OS string descriptor.
pub(crate) const MS_OS_STRING_VENDOR_CODE_OFFSET: usize = 16;

/// Largest bulk or interrupt IN transfer the client allocates a buffer for.
///
/// The server chooses the length of an IN transfer, so it must be bounded before allocating. Windows
/// class drivers stay well below this; WinUSB itself splits transfers above a few megabytes.
pub(crate) const MAX_TRANSFER_LENGTH: u32 = 16 * 1024 * 1024;

/// Budget of one device for its requests in flight, in bytes.
///
/// The server chooses how many requests it keeps outstanding and how large they are. Each request
/// is charged its buffer plus [`REQUEST_OVERHEAD_BYTES`], so this bounds both the memory they pin
/// and their number, at most 65536 per device. Requests beyond the budget fail with
/// `USBD_STATUS_INSUFFICIENT_RESOURCES` before anything is allocated.
pub(crate) const MAX_IN_FLIGHT_BYTES: usize = 64 * 1024 * 1024;

/// Budget charged to every request in flight on top of its buffer, including requests without one.
pub(crate) const REQUEST_OVERHEAD_BYTES: usize = 1024;

/// Timeout for control transfers that carry none, or ask for none.
///
/// Windows lets such a request run until it completes or is cancelled. nusb cannot cancel a
/// submitted control transfer, so a long ceiling stands in for "never": long enough for slow
/// operations such as a firmware download, short enough not to keep a stuck request forever.
pub(crate) const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub(crate) const DESCRIPTOR_TYPE_STRING: u8 = 0x03;

/// The RDPEUSB envelope a request arrived in, which decides the completion shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Envelope {
    /// `TRANSFER_IN_REQUEST`: completed with `URB_COMPLETION` or `URB_COMPLETION_NO_DATA`.
    In { output_buffer_size: u32 },
    /// `TRANSFER_OUT_REQUEST`: completed with `URB_COMPLETION_NO_DATA`.
    Out,
}

/// Endpoint maintenance requested with a `TS_URB_PIPE_REQUEST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipeOperation {
    /// Cancel every transfer pending on the pipe.
    Abort,
    /// Clear the halt condition and reset the data toggle.
    ClearHalt,
    /// Nothing to do on the host side.
    Ignore,
}

/// A host USB operation derived from a `TS_URB`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UsbRequest {
    /// A transfer on the default control pipe.
    ///
    /// For an IN request, `setup.length` is the number of bytes to read. For an OUT request, `data`
    /// holds exactly `setup.length` bytes.
    Control {
        setup: SetupPacket,
        data: Vec<u8>,
        timeout: Duration,
    },
    /// A bulk or interrupt transfer.
    Transfer {
        endpoint: u8,
        direction: Direction,
        /// Number of bytes to read, for an IN transfer.
        length: u32,
        /// Bytes to write, for an OUT transfer.
        data: Vec<u8>,
    },
    /// Read a Microsoft OS feature descriptor.
    OsFeatureDescriptor {
        recipient: Recipient,
        interface_number: u8,
        feature_index: u16,
        length: u16,
    },
    SelectConfiguration {
        /// `None` asks to unconfigure the device.
        configuration_value: Option<u8>,
        interfaces: Vec<TsUsbdInterfaceInfo>,
    },
    SelectInterface {
        interface: TsUsbdInterfaceInfo,
    },
    Pipe {
        endpoint: u8,
        operation: PipeOperation,
    },
    CurrentFrameNumber,
    /// An isochronous transfer, which the host backend cannot perform.
    Isochronous {
        packets: Vec<UsbdIsoPacketDesc>,
    },
    /// A request that is rejected with this status.
    Reject(UsbdStatus),
}

/// Translate the `TS_URB` of a `TRANSFER_IN_REQUEST`.
pub(crate) fn translate_in(packet: TransferInPacket) -> UsbRequest {
    let length = packet.output_buffer_size;
    let func = packet.ts_urb.func;

    match packet.ts_urb.kind {
        TsUrbInKind::SelectConfig(request) => UsbRequest::SelectConfiguration {
            configuration_value: request.desc.map(|desc| desc.configuration_value),
            interfaces: request.usbd_ifaces,
        },
        TsUrbInKind::SelectIface(request) => UsbRequest::SelectInterface {
            interface: request.usbd_iface,
        },
        TsUrbInKind::PipeReq(request) => UsbRequest::Pipe {
            endpoint: pipe_endpoint(request.pipe_handle),
            operation: match func {
                UrbFunction::URB_FUNCTION_ABORT_PIPE => PipeOperation::Abort,
                UrbFunction::URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL
                | UrbFunction::URB_FUNCTION_SYNC_RESET_PIPE
                | UrbFunction::URB_FUNCTION_SYNC_CLEAR_STALL => PipeOperation::ClearHalt,
                _ => PipeOperation::Ignore,
            },
        },
        TsUrbInKind::GetCurFrameNum(_) => UsbRequest::CurrentFrameNumber,
        TsUrbInKind::CtlTransfer(request) => control_transfer(
            request.setup_packet,
            Direction::In,
            length,
            Vec::new(),
            DEFAULT_CONTROL_TIMEOUT,
        ),
        TsUrbInKind::CtlTransferEx(request) => control_transfer(
            request.setup_packet,
            Direction::In,
            length,
            Vec::new(),
            ex_timeout(request.timeout),
        ),
        TsUrbInKind::BulkInterruptTransfer(_) if MAX_TRANSFER_LENGTH < length => {
            UsbRequest::Reject(usbd_status::INVALID_PARAMETER)
        }
        TsUrbInKind::BulkInterruptTransfer(request) => UsbRequest::Transfer {
            endpoint: pipe_endpoint(request.pipe_handle),
            direction: Direction::In,
            length,
            data: Vec::new(),
        },
        TsUrbInKind::IsochTransfer(request) => UsbRequest::Isochronous {
            packets: request.iso_packet,
        },
        TsUrbInKind::CtlDescReq(request) => match function_recipient(func) {
            Some(recipient) => standard(
                Direction::In,
                recipient,
                standard_request::GET_DESCRIPTOR,
                u16::from_be_bytes([request.desc_type, request.index]),
                request.lang_id,
                clamp_length(length),
            ),
            None => UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
        },
        TsUrbInKind::CtlFeatReq(request) => match function_recipient(func) {
            Some(recipient) => {
                let request_code = if is_set_feature(func) {
                    standard_request::SET_FEATURE
                } else {
                    standard_request::CLEAR_FEATURE
                };
                standard(
                    Direction::Out,
                    recipient,
                    request_code,
                    request.feat_selector,
                    request.index,
                    0,
                )
            }
            None => UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
        },
        TsUrbInKind::CtlGetStatus(request) => match function_recipient(func) {
            Some(recipient) => standard(
                Direction::In,
                recipient,
                standard_request::GET_STATUS,
                0,
                request.index,
                clamp_length(length).min(2),
            ),
            None => UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
        },
        TsUrbInKind::VendorClassReq(request) => match (function_kind(func), function_recipient(func)) {
            // Windows also submits OUT requests without a data stage in TRANSFER_IN_REQUEST
            // messages, with transfer flags that keep their direction.
            (Some(kind), Some(recipient)) if request.transfer_flags & USBD_TRANSFER_DIRECTION_IN == 0 => {
                UsbRequest::Control {
                    setup: SetupPacket {
                        request_type: RequestType::new(Direction::Out, kind, recipient),
                        request: request.request,
                        value: request.value,
                        index: request.index,
                        length: 0,
                    },
                    data: Vec::new(),
                    timeout: DEFAULT_CONTROL_TIMEOUT,
                }
            }
            (Some(kind), Some(recipient)) => UsbRequest::Control {
                setup: SetupPacket {
                    request_type: RequestType::new(Direction::In, kind, recipient),
                    request: request.request,
                    value: request.value,
                    index: request.index,
                    length: clamp_length(length),
                },
                data: Vec::new(),
                timeout: DEFAULT_CONTROL_TIMEOUT,
            },
            _ => UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
        },
        TsUrbInKind::CtlGetConfig(_) => standard(
            Direction::In,
            Recipient::DEVICE,
            standard_request::GET_CONFIGURATION,
            0,
            0,
            clamp_length(length).min(1),
        ),
        TsUrbInKind::CtlGetIface(request) => standard(
            Direction::In,
            Recipient::INTERFACE,
            standard_request::GET_INTERFACE,
            0,
            request.interface,
            clamp_length(length).min(1),
        ),
        TsUrbInKind::OsFeatDescReq(request) => match Recipient::from_raw(request.recipient) {
            Some(recipient) => UsbRequest::OsFeatureDescriptor {
                recipient,
                interface_number: request.interface_number,
                feature_index: request.ms_feat_desc_index,
                length: clamp_length(length),
            },
            None => UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
        },
    }
}

/// Translate the `TS_URB` of a `TRANSFER_OUT_REQUEST`.
pub(crate) fn translate_out(packet: TransferOutPacket) -> UsbRequest {
    let data = packet.output_buffer;
    let func = packet.ts_urb.func;
    let Ok(length) = u32::try_from(data.len()) else {
        return UsbRequest::Reject(usbd_status::INVALID_PARAMETER);
    };

    match packet.ts_urb.kind {
        TsUrbOutKind::CtlTransfer(request) => control_transfer(
            request.setup_packet,
            Direction::Out,
            length,
            data,
            DEFAULT_CONTROL_TIMEOUT,
        ),
        TsUrbOutKind::CtlTransferEx(request) => control_transfer(
            request.setup_packet,
            Direction::Out,
            length,
            data,
            ex_timeout(request.timeout),
        ),
        TsUrbOutKind::BulkInterruptTransfer(request) => UsbRequest::Transfer {
            endpoint: pipe_endpoint(request.pipe_handle),
            direction: Direction::Out,
            length,
            data,
        },
        TsUrbOutKind::IsochTransfer(request) => UsbRequest::Isochronous {
            packets: request.iso_packet,
        },
        TsUrbOutKind::CtlDescReq(request) => match (function_recipient(func), u16::try_from(data.len())) {
            (Some(recipient), Ok(length)) => UsbRequest::Control {
                setup: SetupPacket {
                    request_type: RequestType::new(Direction::Out, RequestKind::STANDARD, recipient),
                    request: standard_request::SET_DESCRIPTOR,
                    value: u16::from_be_bytes([request.desc_type, request.index]),
                    index: request.lang_id,
                    length,
                },
                data,
                timeout: DEFAULT_CONTROL_TIMEOUT,
            },
            _ => UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
        },
        TsUrbOutKind::VendorClassReq(request) => {
            match (function_kind(func), function_recipient(func), u16::try_from(data.len())) {
                (Some(kind), Some(recipient), Ok(length)) => UsbRequest::Control {
                    setup: SetupPacket {
                        request_type: RequestType::new(Direction::Out, kind, recipient),
                        request: request.request,
                        value: request.value,
                        index: request.index,
                        length,
                    },
                    data,
                    timeout: DEFAULT_CONTROL_TIMEOUT,
                },
                _ => UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
            }
        }
    }
}

/// Build a control request from a raw setup packet received in `envelope_direction`.
fn control_transfer(
    setup: RdpeusbSetupPacket,
    envelope_direction: Direction,
    envelope_length: u32,
    data: Vec<u8>,
    timeout: Duration,
) -> UsbRequest {
    let request_type = RequestType::from_raw(setup.request_type);

    // The data stage direction is decided by the transfer envelope. The only accepted disagreement is a
    // request without a data stage, which Windows may submit as a TRANSFER_IN_REQUEST.
    let direction = if setup.length == 0 {
        request_type.direction()
    } else {
        envelope_direction
    };
    let length = match direction {
        Direction::In => setup.length.min(clamp_length(envelope_length)),
        Direction::Out => match u16::try_from(data.len()) {
            Ok(length) => length,
            Err(_) => return UsbRequest::Reject(usbd_status::INVALID_PARAMETER),
        },
    };

    UsbRequest::Control {
        setup: SetupPacket {
            request_type: RequestType::new(direction, request_type.kind(), request_type.recipient()),
            request: setup.request,
            value: setup.value,
            index: setup.index,
            length,
        },
        data: if direction == Direction::Out { data } else { Vec::new() },
        timeout,
    }
}

fn standard(
    direction: Direction,
    recipient: Recipient,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
) -> UsbRequest {
    UsbRequest::Control {
        setup: SetupPacket {
            request_type: RequestType::new(direction, RequestKind::STANDARD, recipient),
            request,
            value,
            index,
            length,
        },
        data: Vec::new(),
        timeout: DEFAULT_CONTROL_TIMEOUT,
    }
}

fn ex_timeout(timeout_ms: u32) -> Duration {
    // A zero TimeOut means that the request never times out (see `DEFAULT_CONTROL_TIMEOUT`).
    if timeout_ms == 0 {
        DEFAULT_CONTROL_TIMEOUT
    } else {
        Duration::from_millis(u64::from(timeout_ms))
    }
}

fn clamp_length(length: u32) -> u16 {
    u16::try_from(length).unwrap_or(u16::MAX)
}

fn function_recipient(func: UrbFunction) -> Option<Recipient> {
    match func {
        UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE
        | UrbFunction::URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE
        | UrbFunction::URB_FUNCTION_SET_FEATURE_TO_DEVICE
        | UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_DEVICE
        | UrbFunction::URB_FUNCTION_GET_STATUS_FROM_DEVICE
        | UrbFunction::URB_FUNCTION_VENDOR_DEVICE
        | UrbFunction::URB_FUNCTION_CLASS_DEVICE => Some(Recipient::DEVICE),
        UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE
        | UrbFunction::URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE
        | UrbFunction::URB_FUNCTION_SET_FEATURE_TO_INTERFACE
        | UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_INTERFACE
        | UrbFunction::URB_FUNCTION_GET_STATUS_FROM_INTERFACE
        | UrbFunction::URB_FUNCTION_VENDOR_INTERFACE
        | UrbFunction::URB_FUNCTION_CLASS_INTERFACE => Some(Recipient::INTERFACE),
        UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT
        | UrbFunction::URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT
        | UrbFunction::URB_FUNCTION_SET_FEATURE_TO_ENDPOINT
        | UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT
        | UrbFunction::URB_FUNCTION_GET_STATUS_FROM_ENDPOINT
        | UrbFunction::URB_FUNCTION_VENDOR_ENDPOINT
        | UrbFunction::URB_FUNCTION_CLASS_ENDPOINT => Some(Recipient::ENDPOINT),
        UrbFunction::URB_FUNCTION_SET_FEATURE_TO_OTHER
        | UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_OTHER
        | UrbFunction::URB_FUNCTION_GET_STATUS_FROM_OTHER
        | UrbFunction::URB_FUNCTION_VENDOR_OTHER
        | UrbFunction::URB_FUNCTION_CLASS_OTHER => Some(Recipient::OTHER),
        _ => None,
    }
}

fn function_kind(func: UrbFunction) -> Option<RequestKind> {
    match func {
        UrbFunction::URB_FUNCTION_VENDOR_DEVICE
        | UrbFunction::URB_FUNCTION_VENDOR_INTERFACE
        | UrbFunction::URB_FUNCTION_VENDOR_ENDPOINT
        | UrbFunction::URB_FUNCTION_VENDOR_OTHER => Some(RequestKind::VENDOR),
        UrbFunction::URB_FUNCTION_CLASS_DEVICE
        | UrbFunction::URB_FUNCTION_CLASS_INTERFACE
        | UrbFunction::URB_FUNCTION_CLASS_ENDPOINT
        | UrbFunction::URB_FUNCTION_CLASS_OTHER => Some(RequestKind::CLASS),
        _ => None,
    }
}

fn is_set_feature(func: UrbFunction) -> bool {
    matches!(
        func,
        UrbFunction::URB_FUNCTION_SET_FEATURE_TO_DEVICE
            | UrbFunction::URB_FUNCTION_SET_FEATURE_TO_INTERFACE
            | UrbFunction::URB_FUNCTION_SET_FEATURE_TO_ENDPOINT
            | UrbFunction::URB_FUNCTION_SET_FEATURE_TO_OTHER
    )
}

/// Bytes of buffer the request holds while it is in flight, counted against
/// [`MAX_IN_FLIGHT_BYTES`].
pub(crate) fn buffer_bytes(request: &UsbRequest) -> usize {
    match request {
        UsbRequest::Control { setup, data, .. } => usize::from(setup.length).max(data.len()),
        UsbRequest::Transfer {
            direction: Direction::In,
            length,
            ..
        } => usize::try_from(*length).unwrap_or(usize::MAX),
        UsbRequest::Transfer { data, .. } => data.len(),
        UsbRequest::OsFeatureDescriptor { length, .. } => usize::from(*length),
        UsbRequest::SelectConfiguration { .. }
        | UsbRequest::SelectInterface { .. }
        | UsbRequest::Pipe { .. }
        | UsbRequest::CurrentFrameNumber
        | UsbRequest::Isochronous { .. }
        | UsbRequest::Reject(_) => 0,
    }
}

/// Endpoint address carried in a pipe handle returned by [`select_configuration_result`].
pub(crate) fn pipe_endpoint(pipe_handle: PipeHandle) -> u8 {
    pipe_handle.to_le_bytes()[0]
}

/// How the device worker must execute a control request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlRoute {
    /// Forward the request to the device through a claimed interface.
    Forward,
    /// Read a device-level descriptor through the host's cached path.
    GetDeviceDescriptor {
        descriptor_type: u8,
        descriptor_index: u8,
        language_id: u16,
    },
    /// `SET_CONFIGURATION`, which the host stack owns.
    SetConfiguration(u8),
    /// `SET_INTERFACE`, which must go through the host interface state.
    SetInterface {
        interface_number: u8,
        alternate_setting: u8,
    },
    /// `CLEAR_FEATURE(ENDPOINT_HALT)`, which must also reset the host-side data toggle.
    ClearHalt { endpoint: u8 },
    /// A request the host stack does not let user mode issue.
    Reject,
}

/// Decide how a control request is executed.
///
/// Standard requests that change device state are intercepted, because forwarding them would leave the
/// host USB stack out of sync with the device.
pub(crate) fn route_control(setup: &SetupPacket) -> ControlRoute {
    let [value_low, value_high] = setup.value.to_le_bytes();
    let [index_low, _] = setup.index.to_le_bytes();
    let recipient = setup.request_type.recipient();

    match setup.standard_request() {
        Some(standard_request::GET_DESCRIPTOR)
            if recipient == Recipient::DEVICE && setup.request_type.direction() == Direction::In =>
        {
            ControlRoute::GetDeviceDescriptor {
                descriptor_type: value_high,
                descriptor_index: value_low,
                language_id: setup.index,
            }
        }
        Some(standard_request::SET_CONFIGURATION) => ControlRoute::SetConfiguration(value_low),
        Some(standard_request::SET_INTERFACE) if recipient == Recipient::INTERFACE => ControlRoute::SetInterface {
            interface_number: index_low,
            alternate_setting: value_low,
        },
        Some(standard_request::CLEAR_FEATURE)
            if recipient == Recipient::ENDPOINT && setup.value == standard_feature::ENDPOINT_HALT =>
        {
            ControlRoute::ClearHalt { endpoint: index_low }
        }
        Some(standard_request::SET_ADDRESS) => ControlRoute::Reject,
        _ => ControlRoute::Forward,
    }
}

/// Build the `TS_URB_SELECT_CONFIGURATION_RESULT` for the active configuration.
///
/// Each requested interface is described with its current alternate setting, which the worker has
/// already selected. Pipe information supplied by the server is carried over, as Windows expects.
pub(crate) fn select_configuration_result(
    config_descriptor: &[u8],
    requested: &[TsUsbdInterfaceInfo],
) -> Result<TsUrbSelectConfigResult, UsbdStatus> {
    let set = ConfigurationDescriptorSet::parse(config_descriptor).map_err(|_| usbd_status::REQUEST_FAILED)?;
    let interface = requested
        .iter()
        .map(|interface| interface_result(set, interface))
        .collect::<Result<_, _>>()?;

    Ok(TsUrbSelectConfigResult {
        config_handle: CONFIG_HANDLE_TAG | ConfigHandle::from(set.configuration().configuration_value()),
        interface,
    })
}

/// Build the `TS_URB_SELECT_INTERFACE_RESULT` for one interface.
pub(crate) fn select_interface_result(
    config_descriptor: &[u8],
    requested: &TsUsbdInterfaceInfo,
) -> Result<TsUrbSelectInterfaceResult, UsbdStatus> {
    let set = ConfigurationDescriptorSet::parse(config_descriptor).map_err(|_| usbd_status::REQUEST_FAILED)?;

    Ok(TsUrbSelectInterfaceResult {
        interface: interface_result(set, requested)?,
    })
}

fn interface_result(
    set: ConfigurationDescriptorSet<'_>,
    requested: &TsUsbdInterfaceInfo,
) -> Result<TsUsbdInterfaceInfoResult, UsbdStatus> {
    let descriptor: InterfaceDescriptor<'_> = set
        .interface(requested.interface_number, requested.alternate_setting)
        .ok_or(usbd_status::INVALID_PARAMETER)?;
    let class = descriptor.class();

    let pipes = descriptor
        .endpoints()
        .enumerate()
        .map(|(index, endpoint)| {
            let transfer_type = endpoint.transfer_type();
            let max_packet_size = endpoint.max_packet_size();
            // High-bandwidth periodic endpoints move up to three packets per microframe.
            let packets_per_interval = match transfer_type {
                TransferType::Isochronous | TransferType::Interrupt => {
                    1 + u16::from(max_packet_size.additional_transactions().unwrap_or(0))
                }
                TransferType::Control | TransferType::Bulk => 1,
            };
            let requested_pipe = requested.ts_usbd_pipe_info.get(index);

            TsUsbdPipeInfoResult {
                max_packet_size: max_packet_size.packet_size().saturating_mul(packets_per_interval),
                endpoint_address: endpoint.address_raw(),
                interval: endpoint.interval(),
                pipe_type: match transfer_type {
                    TransferType::Control => UsbdPipeType::Control,
                    TransferType::Isochronous => UsbdPipeType::Isochronous,
                    TransferType::Bulk => UsbdPipeType::Bulk,
                    TransferType::Interrupt => UsbdPipeType::Interrupt,
                },
                pipe_handle: PIPE_HANDLE_TAG | PipeHandle::from(endpoint.address_raw()),
                max_transfer_size: requested_pipe.map_or(u32::MAX, |pipe| pipe.max_transfer_size),
                pipe_flags: requested_pipe.map_or(0, |pipe| pipe.pipe_flags),
            }
        })
        .collect();

    Ok(TsUsbdInterfaceInfoResult {
        interface_number: descriptor.number(),
        alternate_setting: descriptor.alternate_setting(),
        class: class.class,
        sub_class: class.subclass,
        protocol: class.protocol,
        interface_handle: INTERFACE_HANDLE_TAG
            | u32::from(u16::from_be_bytes([
                descriptor.alternate_setting(),
                descriptor.number(),
            ])),
        pipes,
    })
}

pub(crate) fn result(usbd_status: UsbdStatus, payload: Option<TsUrbResultPayload>) -> TsUrbResult<TsUrbResultPayload> {
    TsUrbResult {
        header: TsUrbResultHeader { usbd_status },
        payload,
    }
}

pub(crate) fn hresult(usbd_status: UsbdStatus) -> HResult {
    if usbd_status::is_error(usbd_status) {
        HRESULT_GEN_FAILURE
    } else {
        S_OK
    }
}

/// Complete a request that read `data` (IN) or wrote `written` bytes (OUT).
pub(crate) fn transfer_completion(
    envelope: Envelope,
    usbd_status: UsbdStatus,
    mut data: Vec<u8>,
    written: u32,
) -> CompletionData {
    match envelope {
        Envelope::In { output_buffer_size } => {
            data.truncate(usize::try_from(output_buffer_size).unwrap_or(usize::MAX));
            CompletionData::TransferIn(TransferInCompletionResult {
                ts_urb_result: result(usbd_status, None),
                hresult: hresult(usbd_status),
                output_buffer: data,
            })
        }
        Envelope::Out => CompletionData::TransferOut(TransferOutCompletionResult {
            ts_urb_result: result(usbd_status, None),
            hresult: hresult(usbd_status),
            output_buffer_size: written,
        }),
    }
}

/// Complete a request that returns a `TS_URB_RESULT` payload and no data.
pub(crate) fn payload_completion(envelope: Envelope, payload: TsUrbResultPayload) -> CompletionData {
    status_completion(envelope, usbd_status::SUCCESS, Some(payload))
}

/// Complete a request with `usbd_status` and no data.
pub(crate) fn failure_completion(envelope: Envelope, usbd_status: UsbdStatus) -> CompletionData {
    status_completion(envelope, usbd_status, None)
}

fn status_completion(
    envelope: Envelope,
    usbd_status: UsbdStatus,
    payload: Option<TsUrbResultPayload>,
) -> CompletionData {
    match envelope {
        Envelope::In { .. } => CompletionData::TransferIn(TransferInCompletionResult {
            ts_urb_result: result(usbd_status, payload),
            hresult: hresult(usbd_status),
            output_buffer: Vec::new(),
        }),
        Envelope::Out => CompletionData::TransferOut(TransferOutCompletionResult {
            ts_urb_result: result(usbd_status, payload),
            hresult: hresult(usbd_status),
            output_buffer_size: 0,
        }),
    }
}

/// Reject an isochronous transfer, reporting every packet as not transferred.
pub(crate) fn isochronous_rejection(envelope: Envelope, packets: &[UsbdIsoPacketDesc]) -> CompletionData {
    let packet_status = i32::from_ne_bytes(usbd_status::NOT_SUPPORTED.to_ne_bytes());
    let payload = TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
        start_frame: 0,
        error_count: u32::try_from(packets.len()).unwrap_or(u32::MAX),
        iso_packet: packets
            .iter()
            .map(|packet| UsbdIsoPacketDesc {
                offset: packet.offset,
                length: 0,
                status: packet_status,
            })
            .collect(),
    });

    status_completion(envelope, usbd_status::NOT_SUPPORTED, Some(payload))
}

/// Answer an `IO_CONTROL` request.
///
/// None of these requests reach the device: they describe the upstream port, which the client owns.
pub(crate) fn io_control(packet: &IoControlPacket) -> IoControlCompletionResult {
    match packet.ioctl_code {
        // Resetting or cycling the port would make the device re-enumerate on the client, ending the
        // redirection. Report success, like the FreeRDP client.
        IoctlInternalUsb::ResetPort | IoctlInternalUsb::CyclePort => IoControlCompletionResult {
            hresult: S_OK,
            information: 0,
            output_buffer: Vec::new(),
        },
        IoctlInternalUsb::GetPortStatus if 4 <= packet.output_buffer_size => IoControlCompletionResult {
            hresult: S_OK,
            information: 4,
            output_buffer: PORT_STATUS_ENABLED_CONNECTED.to_le_bytes().to_vec(),
        },
        _ => IoControlCompletionResult {
            hresult: HRESULT_NOT_SUPPORTED,
            information: 0,
            output_buffer: Vec::new(),
        },
    }
}

/// Answer a `QUERY_DEVICE_TEXT` request.
pub(crate) fn device_text(text_type: u32, description: &str, location: &str) -> DeviceText {
    match text_type {
        DEVICE_TEXT_DESCRIPTION => DeviceText {
            hresult: S_OK,
            description: description.to_owned(),
        },
        DEVICE_TEXT_LOCATION_INFORMATION => DeviceText {
            hresult: S_OK,
            description: location.to_owned(),
        },
        _ => DeviceText {
            hresult: HRESULT_NOT_SUPPORTED,
            description: String::new(),
        },
    }
}

/// Extract `bMS_VendorCode` from a Microsoft OS string descriptor.
pub(crate) fn ms_os_vendor_code(descriptor: &[u8]) -> Option<u8> {
    // bDescriptorType, then "MSFT100" in UTF-16LE, after bLength.
    const SIGNATURE: [u8; 15] = [
        DESCRIPTOR_TYPE_STRING,
        b'M',
        0,
        b'S',
        0,
        b'F',
        0,
        b'T',
        0,
        b'1',
        0,
        b'0',
        0,
        b'0',
        0,
    ];

    // The Microsoft OS 1.0 descriptor is 0x12 bytes long. Accept a longer bLength, as one reading of
    // MS-RDPEUSB 3.3.5.3.9 allows, as long as the descriptor reaches bMS_VendorCode.
    let (&length, rest) = descriptor.split_first()?;
    if usize::from(length) <= MS_OS_STRING_VENDOR_CODE_OFFSET || rest.get(..SIGNATURE.len())? != SIGNATURE {
        return None;
    }
    descriptor.get(MS_OS_STRING_VENDOR_CODE_OFFSET).copied()
}

#[cfg(test)]
mod tests {
    use ironrdp_rdpeusb::io::{TsUrbInPacket, TsUrbOutPacket};
    use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::utils::TsUsbdPipeInfo;
    use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::{
        TsUrbBulkOrInterruptTransfer, TsUrbControlDescRequest, TsUrbControlFeatRequest, TsUrbControlTransfer,
        TsUrbControlTransferEx, TsUrbControlVendorClassRequest, TsUrbPipeRequest,
    };

    use super::*;

    fn transfer_in(kind: TsUrbInKind, func: UrbFunction, output_buffer_size: u32) -> TransferInPacket {
        TransferInPacket {
            ts_urb: TsUrbInPacket { kind, func },
            output_buffer_size,
        }
    }

    fn transfer_out(kind: TsUrbOutKind, func: UrbFunction, output_buffer: Vec<u8>) -> TransferOutPacket {
        TransferOutPacket {
            ts_urb: TsUrbOutPacket {
                kind,
                no_ack: false,
                func,
            },
            output_buffer,
        }
    }

    fn control(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> UsbRequest {
        UsbRequest::Control {
            setup: SetupPacket {
                request_type: RequestType::from_raw(request_type),
                request,
                value,
                index,
                length,
            },
            data: Vec::new(),
            timeout: DEFAULT_CONTROL_TIMEOUT,
        }
    }

    // Configuration 1 with interface 0 (vendor class, bulk IN 0x81 and bulk OUT 0x02) and interface 1
    // alternate setting 1 (HID-like, high-bandwidth interrupt IN 0x83).
    const CONFIG: &[u8] = &[
        0x09, 0x02, 0x30, 0x00, 0x02, 0x01, 0x00, 0x80, 0x32, // Configuration
        0x09, 0x04, 0x00, 0x00, 0x02, 0xff, 0x01, 0x02, 0x00, // Interface 0, alt 0
        0x07, 0x05, 0x81, 0x02, 0x00, 0x02, 0x00, // Bulk IN 0x81, 512
        0x07, 0x05, 0x02, 0x02, 0x00, 0x02, 0x00, // Bulk OUT 0x02, 512
        0x09, 0x04, 0x01, 0x01, 0x01, 0x03, 0x00, 0x00, 0x00, // Interface 1, alt 1
        0x07, 0x05, 0x83, 0x03, 0x00, 0x14, 0x04, // Interrupt IN 0x83, 1024 x 3
    ];

    #[test]
    fn control_transfer_in_uses_the_smaller_length() {
        let request = translate_in(transfer_in(
            TsUrbInKind::CtlTransfer(TsUrbControlTransfer {
                pipe: 0,
                transfer_flags: 0x1,
                setup_packet: RdpeusbSetupPacket {
                    request_type: 0xc0,
                    request: 0x42,
                    value: 0x1234,
                    index: 0x0002,
                    length: 64,
                },
            }),
            UrbFunction::URB_FUNCTION_CONTROL_TRANSFER,
            16,
        ));

        assert_eq!(request, control(0xc0, 0x42, 0x1234, 0x0002, 16));
    }

    #[test]
    fn control_transfer_without_data_keeps_its_direction() {
        // SET_FEATURE(DEVICE_REMOTE_WAKEUP) sent in a TRANSFER_IN_REQUEST.
        let request = translate_in(transfer_in(
            TsUrbInKind::CtlTransfer(TsUrbControlTransfer {
                pipe: 0,
                transfer_flags: 0x1,
                setup_packet: RdpeusbSetupPacket {
                    request_type: 0x00,
                    request: standard_request::SET_FEATURE,
                    value: 1,
                    index: 0,
                    length: 0,
                },
            }),
            UrbFunction::URB_FUNCTION_CONTROL_TRANSFER,
            0,
        ));

        assert_eq!(request, control(0x00, standard_request::SET_FEATURE, 1, 0, 0));
    }

    #[test]
    fn control_transfer_out_carries_data_and_timeout() {
        let request = translate_out(transfer_out(
            TsUrbOutKind::CtlTransferEx(TsUrbControlTransferEx {
                pipe: 0,
                transfer_flags: 0,
                timeout: 500,
                setup_packet: RdpeusbSetupPacket {
                    // The envelope wins over a stray IN bit when there is a data stage.
                    request_type: 0xa1,
                    request: 0x09,
                    value: 0x0200,
                    index: 0x0001,
                    length: 3,
                },
            }),
            UrbFunction::URB_FUNCTION_CONTROL_TRANSFER_EX,
            vec![1, 2, 3],
        ));

        assert_eq!(
            request,
            UsbRequest::Control {
                setup: SetupPacket {
                    request_type: RequestType::from_raw(0x21),
                    request: 0x09,
                    value: 0x0200,
                    index: 0x0001,
                    length: 3,
                },
                data: vec![1, 2, 3],
                timeout: Duration::from_millis(500),
            }
        );
    }

    #[test]
    fn descriptor_request_maps_recipient_from_function() {
        let request = translate_in(transfer_in(
            TsUrbInKind::CtlDescReq(TsUrbControlDescRequest {
                index: 2,
                desc_type: 0x03,
                lang_id: 0x0409,
            }),
            UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE,
            255,
        ));

        assert_eq!(
            request,
            control(0x81, standard_request::GET_DESCRIPTOR, 0x0302, 0x0409, 255)
        );
    }

    #[test]
    fn feature_requests_map_set_and_clear() {
        let set = translate_in(transfer_in(
            TsUrbInKind::CtlFeatReq(TsUrbControlFeatRequest {
                feat_selector: 0,
                index: 0x81,
            }),
            UrbFunction::URB_FUNCTION_SET_FEATURE_TO_ENDPOINT,
            0,
        ));
        let clear = translate_in(transfer_in(
            TsUrbInKind::CtlFeatReq(TsUrbControlFeatRequest {
                feat_selector: 0,
                index: 0x81,
            }),
            UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT,
            0,
        ));

        assert_eq!(set, control(0x02, standard_request::SET_FEATURE, 0, 0x81, 0));
        assert_eq!(clear, control(0x02, standard_request::CLEAR_FEATURE, 0, 0x81, 0));
    }

    #[test]
    fn vendor_and_class_requests_map_kind_and_recipient() {
        let vendor_in = translate_in(transfer_in(
            TsUrbInKind::VendorClassReq(TsUrbControlVendorClassRequest {
                transfer_flags: 0x1,
                request: 0x01,
                value: 0x0002,
                index: 0x0003,
            }),
            UrbFunction::URB_FUNCTION_VENDOR_OTHER,
            8,
        ));
        let class_out = translate_out(transfer_out(
            TsUrbOutKind::VendorClassReq(TsUrbControlVendorClassRequest {
                transfer_flags: 0,
                request: 0x22,
                value: 0x0003,
                index: 0x0000,
            }),
            UrbFunction::URB_FUNCTION_CLASS_INTERFACE,
            vec![0xaa],
        ));

        assert_eq!(vendor_in, control(0xc3, 0x01, 0x0002, 0x0003, 8));

        // HID SET_IDLE, an OUT request without data that Windows submits as a TRANSFER_IN_REQUEST.
        let set_idle = translate_in(transfer_in(
            TsUrbInKind::VendorClassReq(TsUrbControlVendorClassRequest {
                transfer_flags: 0,
                request: 0x0a,
                value: 0x0000,
                index: 0x0001,
            }),
            UrbFunction::URB_FUNCTION_CLASS_INTERFACE,
            0,
        ));
        assert_eq!(set_idle, control(0x21, 0x0a, 0x0000, 0x0001, 0));
        assert_eq!(
            class_out,
            UsbRequest::Control {
                setup: SetupPacket {
                    request_type: RequestType::from_raw(0x21),
                    request: 0x22,
                    value: 0x0003,
                    index: 0x0000,
                    length: 1,
                },
                data: vec![0xaa],
                timeout: DEFAULT_CONTROL_TIMEOUT,
            }
        );
    }

    #[test]
    fn bulk_transfers_use_the_endpoint_from_the_pipe_handle() {
        let read = translate_in(transfer_in(
            TsUrbInKind::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
                pipe_handle: PIPE_HANDLE_TAG | 0x81,
                transfer_flags: 0x3,
            }),
            UrbFunction::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER,
            13,
        ));
        let write = translate_out(transfer_out(
            TsUrbOutKind::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
                pipe_handle: PIPE_HANDLE_TAG | 0x02,
                transfer_flags: 0,
            }),
            UrbFunction::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER,
            vec![0x55; 31],
        ));

        assert_eq!(
            read,
            UsbRequest::Transfer {
                endpoint: 0x81,
                direction: Direction::In,
                length: 13,
                data: Vec::new(),
            }
        );
        assert_eq!(
            write,
            UsbRequest::Transfer {
                endpoint: 0x02,
                direction: Direction::Out,
                length: 31,
                data: vec![0x55; 31],
            }
        );
    }

    #[test]
    fn requests_count_their_buffers_against_the_budget() {
        let read = UsbRequest::Transfer {
            endpoint: 0x81,
            direction: Direction::In,
            length: 4096,
            data: Vec::new(),
        };
        let write = UsbRequest::Transfer {
            endpoint: 0x02,
            direction: Direction::Out,
            length: 3,
            data: vec![1, 2, 3],
        };

        assert_eq!(buffer_bytes(&read), 4096);
        assert_eq!(buffer_bytes(&write), 3);
        assert_eq!(buffer_bytes(&control(0xc0, 0x01, 0, 0, 255)), 255);
        assert_eq!(buffer_bytes(&UsbRequest::Reject(usbd_status::INVALID_PARAMETER)), 0);
    }

    #[test]
    fn zero_control_timeout_uses_the_long_default() {
        assert_eq!(ex_timeout(0), DEFAULT_CONTROL_TIMEOUT);
        assert_eq!(ex_timeout(1500), Duration::from_millis(1500));
        assert!(Duration::from_secs(60) < DEFAULT_CONTROL_TIMEOUT);
    }

    #[test]
    fn oversized_in_transfers_are_rejected_before_allocation() {
        let read = |output_buffer_size| {
            translate_in(transfer_in(
                TsUrbInKind::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
                    pipe_handle: PIPE_HANDLE_TAG | 0x81,
                    transfer_flags: 0x3,
                }),
                UrbFunction::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER,
                output_buffer_size,
            ))
        };

        assert_eq!(read(u32::MAX), UsbRequest::Reject(usbd_status::INVALID_PARAMETER));
        assert_eq!(
            read(MAX_TRANSFER_LENGTH + 1),
            UsbRequest::Reject(usbd_status::INVALID_PARAMETER)
        );
        assert!(matches!(
            read(MAX_TRANSFER_LENGTH),
            UsbRequest::Transfer {
                length: MAX_TRANSFER_LENGTH,
                ..
            }
        ));
    }

    #[test]
    fn pipe_requests_map_operations() {
        let pipe = |func| {
            translate_in(transfer_in(
                TsUrbInKind::PipeReq(TsUrbPipeRequest {
                    pipe_handle: PIPE_HANDLE_TAG | 0x83,
                }),
                func,
                0,
            ))
        };

        assert_eq!(
            pipe(UrbFunction::URB_FUNCTION_ABORT_PIPE),
            UsbRequest::Pipe {
                endpoint: 0x83,
                operation: PipeOperation::Abort
            }
        );
        assert_eq!(
            pipe(UrbFunction::URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL),
            UsbRequest::Pipe {
                endpoint: 0x83,
                operation: PipeOperation::ClearHalt
            }
        );
        assert_eq!(
            pipe(UrbFunction::URB_FUNCTION_CLOSE_STATIC_STREAMS),
            UsbRequest::Pipe {
                endpoint: 0x83,
                operation: PipeOperation::Ignore
            }
        );
    }

    #[test]
    fn routes_state_changing_standard_requests() {
        let setup = |request_type: u8, request: u8, value: u16, index: u16| SetupPacket {
            request_type: RequestType::from_raw(request_type),
            request,
            value,
            index,
            length: 0,
        };

        assert_eq!(
            route_control(&setup(0x80, standard_request::GET_DESCRIPTOR, 0x0300, 0x0409)),
            ControlRoute::GetDeviceDescriptor {
                descriptor_type: 0x03,
                descriptor_index: 0,
                language_id: 0x0409
            }
        );
        assert_eq!(
            route_control(&setup(0x00, standard_request::SET_CONFIGURATION, 1, 0)),
            ControlRoute::SetConfiguration(1)
        );
        assert_eq!(
            route_control(&setup(0x01, standard_request::SET_INTERFACE, 2, 1)),
            ControlRoute::SetInterface {
                interface_number: 1,
                alternate_setting: 2
            }
        );
        assert_eq!(
            route_control(&setup(0x02, standard_request::CLEAR_FEATURE, 0, 0x81)),
            ControlRoute::ClearHalt { endpoint: 0x81 }
        );
        assert_eq!(
            route_control(&setup(0x00, standard_request::SET_ADDRESS, 5, 0)),
            ControlRoute::Reject
        );
        // Interface descriptors (HID report descriptors, for example) go to the device.
        assert_eq!(
            route_control(&setup(0x81, standard_request::GET_DESCRIPTOR, 0x2200, 0)),
            ControlRoute::Forward
        );
        assert_eq!(route_control(&setup(0xc0, 0x01, 0, 0)), ControlRoute::Forward);
    }

    #[test]
    fn builds_select_configuration_result() {
        let requested = [
            TsUsbdInterfaceInfo {
                interface_number: 0,
                alternate_setting: 0,
                ts_usbd_pipe_info: vec![
                    TsUsbdPipeInfo {
                        max_packet_size: 0,
                        max_transfer_size: 0x1_0000,
                        pipe_flags: 0,
                    },
                    TsUsbdPipeInfo {
                        max_packet_size: 0,
                        max_transfer_size: 0x2_0000,
                        pipe_flags: 0,
                    },
                ],
            },
            TsUsbdInterfaceInfo {
                interface_number: 1,
                alternate_setting: 1,
                ts_usbd_pipe_info: Vec::new(),
            },
        ];

        let result = select_configuration_result(CONFIG, &requested).expect("valid configuration");

        assert_eq!(result.config_handle, CONFIG_HANDLE_TAG | 1);
        assert_eq!(result.interface.len(), 2);

        let bulk = &result.interface[0];
        assert_eq!((bulk.class, bulk.sub_class, bulk.protocol), (0xff, 0x01, 0x02));
        let endpoints: Vec<_> = bulk
            .pipes
            .iter()
            .map(|pipe| {
                (
                    pipe.endpoint_address,
                    pipe.pipe_type,
                    pipe.max_packet_size,
                    pipe.max_transfer_size,
                )
            })
            .collect();
        assert_eq!(
            endpoints,
            [
                (0x81, UsbdPipeType::Bulk, 512, 0x1_0000),
                (0x02, UsbdPipeType::Bulk, 512, 0x2_0000)
            ]
        );
        assert_eq!(pipe_endpoint(bulk.pipes[0].pipe_handle), 0x81);
        assert_ne!(bulk.pipes[0].pipe_handle, 0);

        let interrupt = &result.interface[1];
        assert_eq!(interrupt.alternate_setting, 1);
        assert_eq!(interrupt.pipes[0].pipe_type, UsbdPipeType::Interrupt);
        assert_eq!(interrupt.pipes[0].max_packet_size, 3 * 1024);
        assert_eq!(interrupt.pipes[0].max_transfer_size, u32::MAX);
        assert_eq!(interrupt.pipes[0].interval, 4);
    }

    #[test]
    fn rejects_unknown_alternate_setting() {
        let requested = TsUsbdInterfaceInfo {
            interface_number: 0,
            alternate_setting: 3,
            ts_usbd_pipe_info: Vec::new(),
        };

        assert_eq!(
            select_interface_result(CONFIG, &requested).map(|_| ()),
            Err(usbd_status::INVALID_PARAMETER)
        );
    }

    #[test]
    fn completions_carry_status_and_hresult() {
        let CompletionData::TransferIn(read) = transfer_completion(
            Envelope::In { output_buffer_size: 2 },
            usbd_status::SUCCESS,
            vec![1, 2, 3],
            0,
        ) else {
            panic!("expected a transfer-in completion");
        };
        assert_eq!(read.output_buffer, [1, 2]);
        assert_eq!(read.hresult, S_OK);

        let CompletionData::TransferOut(stall) = failure_completion(Envelope::Out, usbd_status::STALL_PID) else {
            panic!("expected a transfer-out completion");
        };
        assert_eq!(stall.ts_urb_result.header.usbd_status, usbd_status::STALL_PID);
        assert_eq!(stall.hresult, HRESULT_GEN_FAILURE);
        assert_eq!(stall.output_buffer_size, 0);
    }

    #[test]
    fn isochronous_transfers_are_rejected_per_packet() {
        let packets = [
            UsbdIsoPacketDesc {
                offset: 0,
                length: 0,
                status: 0,
            },
            UsbdIsoPacketDesc {
                offset: 192,
                length: 0,
                status: 0,
            },
        ];

        let CompletionData::TransferIn(completion) = isochronous_rejection(
            Envelope::In {
                output_buffer_size: 384,
            },
            &packets,
        ) else {
            panic!("expected a transfer-in completion");
        };
        let Some(TsUrbResultPayload::Isoch(result)) = completion.ts_urb_result.payload else {
            panic!("expected an isochronous result");
        };
        assert_eq!(result.error_count, 2);
        assert_eq!(result.iso_packet[1].offset, 192);
        assert!(completion.output_buffer.is_empty());
        assert_eq!(completion.ts_urb_result.header.usbd_status, usbd_status::NOT_SUPPORTED);
    }

    #[test]
    fn answers_port_io_controls_locally() {
        let status = io_control(&IoControlPacket {
            ioctl_code: IoctlInternalUsb::GetPortStatus,
            input_buffer: Vec::new(),
            output_buffer_size: 4,
        });
        assert_eq!(status.output_buffer, PORT_STATUS_ENABLED_CONNECTED.to_le_bytes());
        assert_eq!(status.information, 4);

        let reset = io_control(&IoControlPacket {
            ioctl_code: IoctlInternalUsb::ResetPort,
            input_buffer: Vec::new(),
            output_buffer_size: 0,
        });
        assert_eq!(reset.hresult, S_OK);

        let hub_name = io_control(&IoControlPacket {
            ioctl_code: IoctlInternalUsb::GetHubName,
            input_buffer: Vec::new(),
            output_buffer_size: 256,
        });
        assert_eq!(hub_name.hresult, HRESULT_NOT_SUPPORTED);
        assert!(hub_name.output_buffer.is_empty());
    }

    #[test]
    fn reads_ms_os_vendor_code() {
        let mut descriptor = vec![
            0x12, 0x03, b'M', 0, b'S', 0, b'F', 0, b'T', 0, b'1', 0, b'0', 0, b'0', 0, 0x20, 0x00,
        ];
        assert_eq!(ms_os_vendor_code(&descriptor), Some(0x20));

        // A longer bLength is accepted; one too short to hold bMS_VendorCode is not.
        descriptor[0] = 0x14;
        assert_eq!(ms_os_vendor_code(&descriptor), Some(0x20));
        descriptor[0] = 0x10;
        assert_eq!(ms_os_vendor_code(&descriptor), None);
        descriptor[0] = 0x12;

        descriptor[2] = b'X';
        assert_eq!(ms_os_vendor_code(&descriptor), None);
        assert_eq!(ms_os_vendor_code(&[0x12, 0x03]), None);
    }

    #[test]
    fn device_text_types() {
        assert_eq!(device_text(0, "Pico", "Port_#0001.Hub_#0002").description, "Pico");
        assert_eq!(
            device_text(1, "Pico", "Port_#0001.Hub_#0002").description,
            "Port_#0001.Hub_#0002"
        );
        assert_eq!(device_text(7, "Pico", "").hresult, HRESULT_NOT_SUPPORTED);
    }
}
