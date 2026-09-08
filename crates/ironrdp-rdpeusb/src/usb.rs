//! Translation from protocol-independent USB semantics to RDPEUSB requests.
//!
//! This module is deliberately sans-I/O. It does not know about usbredir
//! packets, RDPEUSB request IDs, device state, completion routing, or async
//! execution. Each function returns a complete backend-facing RDPEUSB transfer
//! packet so that the TS_URB payload, URB function, transfer envelope, flags,
//! and buffer shape cannot disagree.
//!
//! The adapter belongs to RDPEUSB because it selects TS_URB forms and
//! Windows USBD conventions; the input types remain protocol-independent.

use alloc::vec::Vec;
use core::fmt;

use crate::{
    io::{
        CompletionData, IoControlPacket, TransferInCompletionResult, TransferInPacket, TransferOutPacket, TsUrbInKind,
        TsUrbInPacket, TsUrbOutKind, TsUrbOutPacket, UrbFunction,
    },
    pdu::{
        completion::ts_urb_result::{TsUrbResultPayload, TsUrbSelectConfigResult, TsUrbSelectInterfaceResult},
        usb_dev::IoctlInternalUsb,
        usb_dev::ts_urb::{
            TsUrbBulkOrInterruptTransfer, TsUrbControlDescRequest, TsUrbControlFeatRequest,
            TsUrbControlGetConfigRequest, TsUrbControlGetInterfaceRequest, TsUrbControlGetStatusRequest,
            TsUrbControlTransfer, TsUrbControlVendorClassRequest, TsUrbGetCurrFrameNum, TsUrbIsochTransfer,
            TsUrbPipeRequest, TsUrbSelectConfig, TsUrbSelectInterface,
            utils::{SetupPacket as RdpeusbSetupPacket, TsUsbdInterfaceInfo, TsUsbdPipeInfo, UsbConfigDesc},
        },
        utils::{ConfigHandle, PipeHandle, UsbdIsoPacketDesc},
    },
};
use ironrdp_usb::{
    Direction, InterfaceSelection, TransferType,
    control::{GetDescriptorRequest, Recipient, RequestKind, SetupPacket, standard_request},
    descriptor::{ConfigurationDescriptorSet, InterfaceDescriptor, ValidConfigurationDescriptorSet},
    transfer::{
        DataTransferRequest, FrameNumber, IsoCompletion, IsochronousPacketCompletion, IsochronousTransferOutput,
        IsochronousTransferRequest, TransferCompletion, UsbError, UsbResult,
    },
};

// WDK USBD transfer flags. RDPEUSB carries these values unchanged in TS_URB.
const USBD_TRANSFER_DIRECTION_IN: u32 = 0x0000_0001;
const USBD_SHORT_TRANSFER_OK: u32 = 0x0000_0002;
const USBD_DEFAULT_PIPE_TRANSFER: u32 = 0x0000_0008;
const USBD_START_ISO_TRANSFER_ASAP: u32 = 0x0000_0004;

// Windows USBD status values carried unchanged in TS_URB_RESULT_HEADER.
const USBD_STATUS_SUCCESS: u32 = 0x0000_0000;
const USBD_STATUS_STALL_PID: u32 = 0xc000_0004;
const USBD_STATUS_BABBLE_DETECTED: u32 = 0xc000_0012;
const USBD_STATUS_CANCELED: u32 = 0xc001_0000;
const USBD_STATUS_TIMEOUT: u32 = 0xc000_6000;
const USBD_STATUS_DEVICE_GONE: u32 = 0xc000_7000;

// MS-ERREF 2.1 defines bit 31 as the HRESULT severity bit.
const HRESULT_FAILURE_BIT: u32 = 0x8000_0000;

// A Windows URB addresses the default control endpoint with a null pipe
// handle and USBD_DEFAULT_PIPE_TRANSFER. This is a Windows URB convention,
// rather than an RDPEUSB-assigned pipe handle.
const DEFAULT_CONTROL_PIPE_HANDLE: PipeHandle = 0;

/// A complete RDPEUSB transfer request, before request-ID allocation and I/O.
#[derive(Debug, Clone)]
pub enum TransferRequest {
    /// An RDPEUSB `TRANSFER_IN_REQUEST` envelope.
    In(TransferInPacket),
    /// An RDPEUSB `TRANSFER_OUT_REQUEST` envelope.
    Out(TransferOutPacket),
}

/// A USB operation that cannot be represented by the requested RDPEUSB form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConversionError {
    InTransferHasData { actual: usize },
    OutTransferLengthMismatch { expected: usize, actual: usize },
    TransferLengthNotRepresentable { length: u32 },
    UnsupportedDescriptorRecipient { recipient: Recipient },
    StatefulStandardRequest { request: u8 },
    InvalidConfigurationHeaderLength { actual: u8 },
    DuplicateInterface { interface: u8 },
    InterfaceNotFound { selection: InterfaceSelection },
    MissingInterfaceSelection { interface: u8 },
    InvalidMaximumPacketSize { selection: InterfaceSelection, raw: u16 },
    EmptyIsochronousTransfer,
    IsochronousTransferLengthOverflow { packet: usize },
    NoAckIsochronousIn,
}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InTransferHasData { actual } => {
                write!(f, "USB IN transfer carries {actual} bytes of output data")
            }
            Self::OutTransferLengthMismatch { expected, actual } => write!(
                f,
                "USB OUT transfer declares {expected} bytes but carries {actual} bytes"
            ),
            Self::TransferLengthNotRepresentable { length } => {
                write!(f, "USB transfer length {length} does not fit in usize")
            }
            Self::UnsupportedDescriptorRecipient { recipient } => write!(
                f,
                "RDPEUSB has no typed descriptor request for USB recipient {}",
                recipient.raw()
            ),
            Self::StatefulStandardRequest { request } => write!(
                f,
                "USB standard request {:#04x} requires host-controller state and cannot use a generic RDPEUSB control transfer",
                request
            ),
            Self::InvalidConfigurationHeaderLength { actual } => write!(
                f,
                "RDPEUSB requires a 9-byte USB configuration header, got {actual} bytes"
            ),
            Self::DuplicateInterface { interface } => {
                write!(f, "USB interface {interface} is selected more than once")
            }
            Self::InterfaceNotFound { selection } => write!(
                f,
                "USB interface {} alternate setting {} is absent from the configuration",
                selection.interface, selection.alternate_setting
            ),
            Self::MissingInterfaceSelection { interface } => write!(
                f,
                "USB configuration has no selected alternate setting for interface {}",
                interface
            ),
            Self::InvalidMaximumPacketSize { selection, raw } => write!(
                f,
                "USB interface {} alternate setting {} has invalid wMaxPacketSize {raw:#06x}",
                selection.interface, selection.alternate_setting
            ),
            Self::EmptyIsochronousTransfer => f.write_str("USB isochronous transfer contains no packets"),
            Self::IsochronousTransferLengthOverflow { packet } => {
                write!(f, "USB isochronous transfer length overflows u32 at packet {packet}")
            }
            Self::NoAckIsochronousIn => f.write_str("RDPEUSB NoAck is not valid for an isochronous IN transfer"),
        }
    }
}

impl core::error::Error for ConversionError {}

/// An RDPEUSB completion that does not match the submitted USB operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompletionError {
    ExpectedTransferIn,
    ExpectedTransfer,
    ExpectedIoControl,
    UnexpectedUrbResultPayload,
    MissingSelectConfigurationResult,
    MissingSelectInterfaceResult,
    MissingCurrentFrameNumberResult,
    MissingIsochronousResult,
    UnexpectedOutputData { actual: usize },
    ExpectedOneByteOutput { actual: usize },
    OutputLengthTooLarge { actual: usize },
    IsochronousPacketCountMismatch { expected: usize, actual: usize },
    IsochronousPacketLengthExceeded { packet: usize, requested: u32, actual: u32 },
    IsochronousTransferLengthOverflow,
    IsochronousDataLengthMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for CompletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedTransferIn => f.write_str("expected an RDPEUSB transfer-in completion"),
            Self::ExpectedTransfer => f.write_str("expected an RDPEUSB transfer completion"),
            Self::ExpectedIoControl => f.write_str("expected an RDPEUSB IO-control completion"),
            Self::UnexpectedUrbResultPayload => {
                f.write_str("RDPEUSB completion contains an unexpected TS_URB result payload")
            }
            Self::MissingSelectConfigurationResult => {
                f.write_str("RDPEUSB completion does not contain a select-configuration result")
            }
            Self::MissingSelectInterfaceResult => {
                f.write_str("RDPEUSB completion does not contain a select-interface result")
            }
            Self::MissingCurrentFrameNumberResult => {
                f.write_str("RDPEUSB completion does not contain a current-frame-number result")
            }
            Self::MissingIsochronousResult => {
                f.write_str("RDPEUSB completion does not contain an isochronous-transfer result")
            }
            Self::UnexpectedOutputData { actual } => {
                write!(f, "RDPEUSB completion unexpectedly contains {actual} output bytes")
            }
            Self::ExpectedOneByteOutput { actual } => {
                write!(f, "RDPEUSB completion contains {actual} output bytes instead of one")
            }
            Self::OutputLengthTooLarge { actual } => {
                write!(f, "RDPEUSB completion output length {actual} does not fit in u32")
            }
            Self::IsochronousPacketCountMismatch { expected, actual } => write!(
                f,
                "RDPEUSB isochronous completion contains {actual} packets instead of {expected}"
            ),
            Self::IsochronousPacketLengthExceeded {
                packet,
                requested,
                actual,
            } => write!(
                f,
                "RDPEUSB isochronous packet {packet} returned {actual} bytes for a {requested}-byte request"
            ),
            Self::IsochronousTransferLengthOverflow => {
                f.write_str("RDPEUSB isochronous completion length overflows u32")
            }
            Self::IsochronousDataLengthMismatch { expected, actual } => write!(
                f,
                "RDPEUSB isochronous completion carries {actual} bytes instead of {expected}"
            ),
        }
    }
}

impl core::error::Error for CompletionError {}

/// Build a typed RDPEUSB GET_DESCRIPTOR request.
///
/// RDPEUSB only defines typed descriptor URBs for device, interface, and
/// endpoint recipients. [`control_transfer`] falls back to a generic control
/// URB for other recipients, while this explicitly typed helper returns an
/// error.
pub fn get_descriptor(request: GetDescriptorRequest) -> Result<TransferInPacket, ConversionError> {
    let func = descriptor_function(request.recipient, DescriptorOperation::Get).ok_or(
        ConversionError::UnsupportedDescriptorRecipient {
            recipient: request.recipient,
        },
    )?;
    let urb = TsUrbControlDescRequest {
        index: request.descriptor_index,
        desc_type: request.descriptor_type,
        lang_id: request.index,
    };

    Ok(transfer_in(
        TsUrbInKind::CtlDescReq(urb),
        func,
        u32::from(request.requested_length),
    ))
}

/// Translate an RDPEUSB `GET_DESCRIPTOR` completion into general USB semantics.
///
/// A USB operation failure is returned as the [`UsbResult`] error. This
/// function returns [`CompletionError`] only when the completion envelope does
/// not match the submitted operation.
///
/// [MS-RDPEUSB 2.2.9.9] requires descriptor reads to use
/// `TRANSFER_IN_REQUEST`; the corresponding result has only
/// `TS_URB_RESULT_HEADER` plus the optional data buffer carried by
/// [`URB_COMPLETION`][MS-RDPEUSB 2.2.7.2].
///
/// [MS-RDPEUSB 2.2.9.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125
/// [MS-RDPEUSB 2.2.7.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/5bfa9c84-a74b-4942-9d09-e770b21081eb
pub fn get_descriptor_completion(completion: CompletionData) -> Result<UsbResult<Vec<u8>>, CompletionError> {
    let completion = header_only_transfer_in(completion)?;

    let status = completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status);
    Ok(status.map(|()| completion.output_buffer))
}

/// Translate a header-only RDPEUSB transfer completion into general USB data
/// transfer semantics.
///
/// This covers control, bulk, and interrupt transfers. The completion
/// envelope determines the direction: transfer-in data is returned verbatim,
/// while transfer-out reports its completed byte count and an empty data
/// buffer.
pub fn transfer_completion(completion: CompletionData) -> Result<TransferCompletion<Vec<u8>>, CompletionError> {
    match completion {
        CompletionData::TransferIn(completion) => {
            if completion.ts_urb_result.payload.is_some() {
                return Err(CompletionError::UnexpectedUrbResultPayload);
            }
            let actual_length =
                u32::try_from(completion.output_buffer.len()).map_err(|_| CompletionError::OutputLengthTooLarge {
                    actual: completion.output_buffer.len(),
                })?;

            Ok(TransferCompletion {
                status: completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status),
                actual_length,
                data: completion.output_buffer,
            })
        }
        CompletionData::TransferOut(completion) => {
            if completion.ts_urb_result.payload.is_some() {
                return Err(CompletionError::UnexpectedUrbResultPayload);
            }

            Ok(TransferCompletion {
                status: completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status),
                actual_length: completion.output_buffer_size,
                data: Vec::new(),
            })
        }
        CompletionData::IoControl(_) | CompletionData::InternalIoControl(_) => Err(CompletionError::ExpectedTransfer),
    }
}

/// Build an RDPEUSB `GET_CONFIGURATION` request.
#[must_use]
pub fn get_configuration() -> TransferInPacket {
    transfer_in(
        TsUrbInKind::CtlGetConfig(TsUrbControlGetConfigRequest),
        UrbFunction::URB_FUNCTION_GET_CONFIGURATION,
        1,
    )
}

/// Translate an RDPEUSB `GET_CONFIGURATION` completion.
pub fn get_configuration_completion(completion: CompletionData) -> Result<UsbResult<u8>, CompletionError> {
    byte_transfer_in_completion(completion)
}

/// Build an RDPEUSB `GET_INTERFACE` request.
#[must_use]
pub fn get_interface(interface: u8) -> TransferInPacket {
    transfer_in(
        TsUrbInKind::CtlGetIface(TsUrbControlGetInterfaceRequest {
            interface: u16::from(interface),
        }),
        UrbFunction::URB_FUNCTION_GET_INTERFACE,
        1,
    )
}

/// Translate an RDPEUSB `GET_INTERFACE` completion.
pub fn get_interface_completion(completion: CompletionData) -> Result<UsbResult<u8>, CompletionError> {
    byte_transfer_in_completion(completion)
}

/// Translate a default-control-pipe USB request into an RDPEUSB transfer.
///
/// Canonical standard, class, and vendor requests use the operation-specific
/// TS_URB forms from MS-RDPEUSB sections 2.2.9.9 through 2.2.9.14. Other setup
/// packets are preserved losslessly in `TS_URB_CONTROL_TRANSFER`. Requests
/// whose execution changes host-controller state must be handled by the
/// owning state layer instead of this function.
pub fn control_transfer(setup: SetupPacket, data: Vec<u8>) -> Result<TransferRequest, ConversionError> {
    validate_control_data(setup, &data)?;
    if let Some(request) = setup.standard_request() {
        if matches!(
            request,
            standard_request::SET_ADDRESS | standard_request::SET_CONFIGURATION | standard_request::SET_INTERFACE
        ) {
            return Err(ConversionError::StatefulStandardRequest { request });
        }
    }

    let data = if setup.request_type.kind() == RequestKind::STANDARD {
        match standard_control_transfer(setup, data) {
            Ok(request) => return Ok(request),
            Err(data) => data,
        }
    } else if matches!(setup.request_type.kind(), RequestKind::CLASS | RequestKind::VENDOR) {
        if let Some(func) = vendor_or_class_function(setup.request_type.kind(), setup.request_type.recipient()) {
            let urb = TsUrbControlVendorClassRequest {
                transfer_flags: in_transfer_flags(setup.request_type.direction()),
                request: setup.request,
                value: setup.value,
                index: setup.index,
            };
            return Ok(match setup.request_type.direction() {
                Direction::In => TransferRequest::In(transfer_in(
                    TsUrbInKind::VendorClassReq(urb),
                    func,
                    u32::from(setup.length),
                )),
                Direction::Out => TransferRequest::Out(transfer_out(TsUrbOutKind::VendorClassReq(urb), func, data)),
            });
        }
        data
    } else {
        data
    };

    Ok(generic_control_transfer(setup, data))
}

/// Build an RDPEUSB bulk or interrupt IN transfer.
#[must_use]
fn bulk_or_interrupt_in(pipe_handle: PipeHandle, requested_length: u32) -> TransferInPacket {
    transfer_in(
        TsUrbInKind::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
            pipe_handle,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN | USBD_SHORT_TRANSFER_OK,
        }),
        UrbFunction::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER,
        requested_length,
    )
}

/// Build an RDPEUSB bulk or interrupt OUT transfer.
#[must_use]
fn bulk_or_interrupt_out(pipe_handle: PipeHandle, data: Vec<u8>) -> TransferOutPacket {
    transfer_out(
        TsUrbOutKind::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
            pipe_handle,
            transfer_flags: 0,
        }),
        UrbFunction::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER,
        data,
    )
}

/// Build an RDPEUSB bulk or interrupt transfer from general USB semantics.
pub fn bulk_or_interrupt(
    pipe_handle: PipeHandle,
    request: DataTransferRequest<Vec<u8>>,
) -> Result<TransferRequest, ConversionError> {
    match request.endpoint.direction() {
        Direction::In => {
            if !request.data.is_empty() {
                return Err(ConversionError::InTransferHasData {
                    actual: request.data.len(),
                });
            }

            Ok(TransferRequest::In(bulk_or_interrupt_in(pipe_handle, request.length)))
        }
        Direction::Out => {
            let expected = usize::try_from(request.length)
                .map_err(|_| ConversionError::TransferLengthNotRepresentable { length: request.length })?;
            if request.data.len() != expected {
                return Err(ConversionError::OutTransferLengthMismatch {
                    expected,
                    actual: request.data.len(),
                });
            }

            Ok(TransferRequest::Out(bulk_or_interrupt_out(pipe_handle, request.data)))
        }
    }
}

/// Build an RDPEUSB isochronous transfer.
///
/// Packet lengths are translated into the prefix offsets required by
/// `USBD_ISO_PACKET_DESCRIPTOR`. Its request-side `Length` and `Status` fields
/// are output-only and are therefore set to zero.
pub fn isochronous(
    pipe_handle: PipeHandle,
    request: IsochronousTransferRequest<Vec<u8>, Vec<u32>>,
    no_ack: bool,
) -> Result<TransferRequest, ConversionError> {
    let IsochronousTransferRequest {
        endpoint,
        start_frame,
        data,
        packets,
    } = request;
    if packets.is_empty() {
        return Err(ConversionError::EmptyIsochronousTransfer);
    }

    let mut total_length = 0u32;
    let mut iso_packet = Vec::with_capacity(packets.len());
    for (packet, length) in packets.into_iter().enumerate() {
        iso_packet.push(UsbdIsoPacketDesc {
            offset: total_length,
            length: 0,
            status: 0,
        });
        total_length = total_length
            .checked_add(length)
            .ok_or(ConversionError::IsochronousTransferLengthOverflow { packet })?;
    }

    let direction = endpoint.direction();
    if no_ack && direction == Direction::In {
        return Err(ConversionError::NoAckIsochronousIn);
    }
    match direction {
        Direction::In if !data.is_empty() => {
            return Err(ConversionError::InTransferHasData { actual: data.len() });
        }
        Direction::Out => {
            let expected = usize::try_from(total_length)
                .map_err(|_| ConversionError::TransferLengthNotRepresentable { length: total_length })?;
            if data.len() != expected {
                return Err(ConversionError::OutTransferLengthMismatch {
                    expected,
                    actual: data.len(),
                });
            }
        }
        Direction::In => {}
    }

    let transfer_flags = match direction {
        Direction::In => USBD_TRANSFER_DIRECTION_IN,
        Direction::Out => 0,
    } | if start_frame.is_none() {
        USBD_START_ISO_TRANSFER_ASAP
    } else {
        0
    };
    let urb = TsUrbIsochTransfer {
        pipe_handle,
        transfer_flags,
        start_frame: start_frame.unwrap_or(0),
        error_count: 0,
        iso_packet,
    };

    Ok(match direction {
        Direction::In => TransferRequest::In(transfer_in(
            TsUrbInKind::IsochTransfer(urb),
            UrbFunction::URB_FUNCTION_ISOCH_TRANSFER,
            total_length,
        )),
        Direction::Out => {
            let mut packet = transfer_out(
                TsUrbOutKind::IsochTransfer(urb),
                UrbFunction::URB_FUNCTION_ISOCH_TRANSFER,
                data,
            );
            packet.ts_urb.no_ack = no_ack;
            TransferRequest::Out(packet)
        }
    })
}

/// Build an RDPEUSB unconfigure request selecting configuration zero.
#[must_use]
pub fn unconfigure() -> TransferInPacket {
    transfer_in(
        TsUrbInKind::SelectConfig(TsUrbSelectConfig {
            usbd_ifaces: Vec::new(),
            desc: None,
        }),
        UrbFunction::URB_FUNCTION_SELECT_CONFIGURATION,
        0,
    )
}

/// Build an RDPEUSB select-configuration request.
///
/// The complete descriptor set is included, and `active_interfaces` determines
/// the alternate setting requested for every interface number. The interface
/// array preserves caller order. Configuration zero is selected by
/// [`unconfigure`] instead.
///
/// `TS_URB_SELECT_CONFIGURATION` carries the descriptor's declared counts
/// verbatim next to a pipe array derived from the descriptors actually
/// present, so a validated set is required to keep the two consistent.
pub fn select_configuration(
    descriptor: ValidConfigurationDescriptorSet<'_>,
    active_interfaces: &[InterfaceSelection],
) -> Result<TransferInPacket, ConversionError> {
    let descriptor = descriptor.as_set();

    let mut usbd_ifaces = Vec::with_capacity(active_interfaces.len());
    for (index, selection) in active_interfaces.iter().copied().enumerate() {
        if active_interfaces[..index]
            .iter()
            .any(|previous| previous.interface == selection.interface)
        {
            return Err(ConversionError::DuplicateInterface {
                interface: selection.interface,
            });
        }
        let interface = descriptor
            .interface(selection.interface, selection.alternate_setting)
            .ok_or(ConversionError::InterfaceNotFound { selection })?;
        usbd_ifaces.push(interface_information(interface)?);
    }
    for interface in descriptor.interfaces() {
        if !active_interfaces
            .iter()
            .any(|selection| selection.interface == interface.number())
        {
            return Err(ConversionError::MissingInterfaceSelection {
                interface: interface.number(),
            });
        }
    }
    Ok(transfer_in(
        TsUrbInKind::SelectConfig(TsUrbSelectConfig {
            usbd_ifaces,
            desc: Some(configuration_descriptor(descriptor)?),
        }),
        UrbFunction::URB_FUNCTION_SELECT_CONFIGURATION,
        0,
    ))
}

/// Build an RDPEUSB select-interface request.
///
/// `descriptor` must describe the configuration `config_handle` refers to, and
/// `selection` names the interface number and the alternate setting to
/// activate. The request carries no descriptor bytes: the client matches the
/// pipe array against the descriptor it retained from the earlier
/// select-configuration, which is why a validated set is required here too.
pub fn select_interface(
    config_handle: ConfigHandle,
    descriptor: ValidConfigurationDescriptorSet<'_>,
    selection: InterfaceSelection,
) -> Result<TransferInPacket, ConversionError> {
    let interface = descriptor
        .as_set()
        .interface(selection.interface, selection.alternate_setting)
        .ok_or(ConversionError::InterfaceNotFound { selection })?;
    let urb = TsUrbSelectInterface {
        config_handle,
        usbd_iface: interface_information(interface)?,
    };
    Ok(transfer_in(
        TsUrbInKind::SelectIface(urb),
        UrbFunction::URB_FUNCTION_SELECT_INTERFACE,
        0,
    ))
}

/// Translate an RDPEUSB select-configuration completion.
///
/// The returned configuration and pipe handles are RDPEUSB-private opaque
/// values. A server facade is expected to validate and retain them, then
/// expose only the general USB success or failure to its caller.
pub fn select_configuration_completion(
    completion: CompletionData,
) -> Result<UsbResult<TsUrbSelectConfigResult>, CompletionError> {
    let CompletionData::TransferIn(completion) = completion else {
        return Err(CompletionError::ExpectedTransferIn);
    };
    ensure_empty_output(&completion)?;
    let Some(TsUrbResultPayload::SelectConfig(result)) = completion.ts_urb_result.payload else {
        return Err(CompletionError::MissingSelectConfigurationResult);
    };

    let status = completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status);
    Ok(status.map(|()| result))
}

/// Translate an RDPEUSB select-interface completion.
pub fn select_interface_completion(
    completion: CompletionData,
) -> Result<UsbResult<TsUrbSelectInterfaceResult>, CompletionError> {
    let CompletionData::TransferIn(completion) = completion else {
        return Err(CompletionError::ExpectedTransferIn);
    };
    ensure_empty_output(&completion)?;
    let Some(TsUrbResultPayload::SelectIface(result)) = completion.ts_urb_result.payload else {
        return Err(CompletionError::MissingSelectInterfaceResult);
    };

    let status = completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status);
    Ok(status.map(|()| result))
}

/// Build a pipe reset that also clears the endpoint stall state.
///
/// [MS-RDPEUSB 2.2.9.4] carries `URB_PIPE_REQUEST` in a transfer-in request
/// with an empty output buffer.
///
/// [MS-RDPEUSB 2.2.9.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/dcba564e-de14-4d60-82ac-a0fe7a52b312
#[must_use]
pub fn reset_pipe_and_clear_stall(pipe_handle: PipeHandle) -> TransferInPacket {
    transfer_in(
        TsUrbInKind::PipeReq(TsUrbPipeRequest { pipe_handle }),
        UrbFunction::URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL,
        0,
    )
}

/// Translate a header-only RDPEUSB pipe-request completion.
pub fn pipe_request_completion(completion: CompletionData) -> Result<UsbResult<()>, CompletionError> {
    unit_transfer_in_completion(completion)
}

/// Build an RDPEUSB current-frame-number request.
///
/// [MS-RDPEUSB 2.2.9.5] carries this URB in a transfer-in request whose output
/// buffer size is zero; the frame number is part of the TS_URB result.
///
/// [MS-RDPEUSB 2.2.9.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/4985b1dc-5bd9-4988-97a6-063969dc26b4
#[must_use]
pub fn current_frame_number() -> TransferInPacket {
    transfer_in(
        TsUrbInKind::GetCurFrameNum(TsUrbGetCurrFrameNum),
        UrbFunction::URB_FUNCTION_GET_CURRENT_FRAME_NUMBER,
        0,
    )
}

/// Translate an RDPEUSB current-frame-number completion.
pub fn current_frame_number_completion(completion: CompletionData) -> Result<UsbResult<FrameNumber>, CompletionError> {
    let CompletionData::TransferIn(completion) = completion else {
        return Err(CompletionError::ExpectedTransferIn);
    };
    ensure_empty_output(&completion)?;
    let Some(TsUrbResultPayload::FrameNum(result)) = completion.ts_urb_result.payload else {
        return Err(CompletionError::MissingCurrentFrameNumberResult);
    };

    let status = completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status);
    Ok(status.map(|()| result.frame_number))
}

/// Build an RDPEUSB upstream-port reset request.
///
/// [MS-RDPEUSB 2.2.12.1] requires an `IO_CONTROL` request with empty input and
/// output buffers.
///
/// [MS-RDPEUSB 2.2.12.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/8f13c014-2ece-481d-a843-9ae9b03d45fe
#[must_use]
pub fn reset_device() -> IoControlPacket {
    IoControlPacket {
        ioctl_code: IoctlInternalUsb::ResetPort,
        input_buffer: Vec::new(),
        output_buffer_size: 0,
    }
}

/// Translate an RDPEUSB upstream-port reset completion.
pub fn reset_device_completion(completion: CompletionData) -> Result<UsbResult<()>, CompletionError> {
    let CompletionData::IoControl(completion) = completion else {
        return Err(CompletionError::ExpectedIoControl);
    };
    if !completion.output_buffer.is_empty() {
        return Err(CompletionError::UnexpectedOutputData {
            actual: completion.output_buffer.len(),
        });
    }

    if hresult_succeeded(completion.hresult) {
        Ok(Ok(()))
    } else {
        Ok(Err(UsbError::Error))
    }
}

/// Translate an acknowledged RDPEUSB isochronous completion.
///
/// For transfer-in, [MS-RDPEUSB 2.2.10.5] packs only successful packet data
/// into the output buffer. Packet offsets continue to describe the original
/// transfer buffer and therefore are not used to slice that packed data.
///
/// [MS-RDPEUSB 2.2.10.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6f072673-52b8-4750-ac91-9d2313f13b17
pub fn isochronous_completion(
    completion: CompletionData,
    requested_packet_lengths: &[u32],
) -> Result<IsoCompletion<Vec<u8>, Vec<IsochronousPacketCompletion>>, CompletionError> {
    let (ts_urb_result, hresult, output_buffer, envelope_actual_length, transfer_in) = match completion {
        CompletionData::TransferIn(completion) => {
            let actual_length =
                u32::try_from(completion.output_buffer.len()).map_err(|_| CompletionError::OutputLengthTooLarge {
                    actual: completion.output_buffer.len(),
                })?;
            (
                completion.ts_urb_result,
                completion.hresult,
                completion.output_buffer,
                actual_length,
                true,
            )
        }
        CompletionData::TransferOut(completion) => (
            completion.ts_urb_result,
            completion.hresult,
            Vec::new(),
            completion.output_buffer_size,
            false,
        ),
        CompletionData::IoControl(_) | CompletionData::InternalIoControl(_) => {
            return Err(CompletionError::ExpectedTransfer);
        }
    };
    let Some(TsUrbResultPayload::Isoch(result)) = ts_urb_result.payload else {
        return Err(CompletionError::MissingIsochronousResult);
    };

    // Clients such as FreeRDP return an empty packet list when the overall
    // transfer failed. Packet details are meaningful only for a successful
    // URB, so preserve the USB failure instead of upgrading it to a shape
    // error.
    let status = completion_status(hresult, ts_urb_result.header.usbd_status);
    if let Err(error) = status {
        return Ok(Err(error));
    }

    if result.iso_packet.len() != requested_packet_lengths.len() {
        return Err(CompletionError::IsochronousPacketCountMismatch {
            expected: requested_packet_lengths.len(),
            actual: result.iso_packet.len(),
        });
    }

    let mut successful_length = 0u32;
    let mut packets = Vec::with_capacity(result.iso_packet.len());
    for (packet, (result, requested)) in result
        .iso_packet
        .into_iter()
        .zip(requested_packet_lengths.iter().copied())
        .enumerate()
    {
        if result.length > requested {
            return Err(CompletionError::IsochronousPacketLengthExceeded {
                packet,
                requested,
                actual: result.length,
            });
        }

        let packet_status = usb_status(packet_status_raw(result.status));
        if packet_status.is_ok() {
            successful_length = successful_length
                .checked_add(result.length)
                .ok_or(CompletionError::IsochronousTransferLengthOverflow)?;
        }
        packets.push(IsochronousPacketCompletion {
            status: packet_status,
            actual_length: result.length,
        });
    }

    // Only transfer-in completions contain the packed successful-packet data
    // described by MS-RDPEUSB 2.2.10.5. For transfer-out, OutputBufferSize is
    // the number of bytes sent and is not required to equal that packet sum.
    if transfer_in && envelope_actual_length != successful_length {
        return Err(CompletionError::IsochronousDataLengthMismatch {
            expected: successful_length,
            actual: envelope_actual_length,
        });
    }
    Ok(Ok(IsochronousTransferOutput {
        start_frame: result.start_frame,
        actual_length: envelope_actual_length,
        data: output_buffer,
        packets,
    }))
}

fn standard_control_transfer(setup: SetupPacket, data: Vec<u8>) -> Result<TransferRequest, Vec<u8>> {
    let Some(standard_request) = setup.standard_request() else {
        return Err(data);
    };
    let recipient = setup.request_type.recipient();
    let direction = setup.request_type.direction();

    match standard_request {
        standard_request::GET_DESCRIPTOR => {
            // The setup packet and the recipient are already checked by the
            // typed request; an unsupported one falls back to a generic
            // control URB with the caller's buffer intact.
            let Ok(request) = GetDescriptorRequest::try_from(setup) else {
                return Err(data);
            };
            let Ok(packet) = get_descriptor(request) else {
                return Err(data);
            };
            Ok(TransferRequest::In(packet))
        }
        standard_request::SET_DESCRIPTOR if setup.length == 0 || direction == Direction::Out => {
            let Some(func) = descriptor_function(recipient, DescriptorOperation::Set) else {
                return Err(data);
            };
            let [index, desc_type] = setup.value.to_le_bytes();
            Ok(TransferRequest::Out(transfer_out(
                TsUrbOutKind::CtlDescReq(TsUrbControlDescRequest {
                    index,
                    desc_type,
                    lang_id: setup.index,
                }),
                func,
                data,
            )))
        }
        standard_request::GET_STATUS if direction == Direction::In && setup.value == 0 && setup.length == 2 => {
            let Some(func) = get_status_function(recipient) else {
                return Err(data);
            };
            Ok(TransferRequest::In(transfer_in(
                TsUrbInKind::CtlGetStatus(TsUrbControlGetStatusRequest { index: setup.index }),
                func,
                2,
            )))
        }
        standard_request::CLEAR_FEATURE | standard_request::SET_FEATURE
            if direction == Direction::Out && setup.length == 0 =>
        {
            let Some(func) = feature_function(standard_request, recipient) else {
                return Err(data);
            };
            // MS-RDPEUSB 2.2.9.10 requires this USB host-to-device operation
            // in TRANSFER_IN_REQUEST with an empty output buffer.
            Ok(TransferRequest::In(transfer_in(
                TsUrbInKind::CtlFeatReq(TsUrbControlFeatRequest {
                    feat_selector: setup.value,
                    index: setup.index,
                }),
                func,
                0,
            )))
        }
        standard_request::GET_CONFIGURATION
            if direction == Direction::In
                && recipient == Recipient::DEVICE
                && setup.value == 0
                && setup.index == 0
                && setup.length == 1 =>
        {
            Ok(TransferRequest::In(transfer_in(
                TsUrbInKind::CtlGetConfig(TsUrbControlGetConfigRequest),
                UrbFunction::URB_FUNCTION_GET_CONFIGURATION,
                1,
            )))
        }
        standard_request::GET_INTERFACE
            if direction == Direction::In
                && recipient == Recipient::INTERFACE
                && setup.value == 0
                && setup.length == 1 =>
        {
            Ok(TransferRequest::In(transfer_in(
                TsUrbInKind::CtlGetIface(TsUrbControlGetInterfaceRequest { interface: setup.index }),
                UrbFunction::URB_FUNCTION_GET_INTERFACE,
                1,
            )))
        }
        _ => Err(data),
    }
}

fn generic_control_transfer(setup: SetupPacket, data: Vec<u8>) -> TransferRequest {
    let direction = setup.request_type.direction();
    let urb = TsUrbControlTransfer {
        pipe: DEFAULT_CONTROL_PIPE_HANDLE,
        transfer_flags: in_transfer_flags(direction) | USBD_DEFAULT_PIPE_TRANSFER,
        setup_packet: rdpeusb_setup_packet(setup),
    };

    match direction {
        Direction::In => TransferRequest::In(transfer_in(
            TsUrbInKind::CtlTransfer(urb),
            UrbFunction::URB_FUNCTION_CONTROL_TRANSFER,
            u32::from(setup.length),
        )),
        Direction::Out => TransferRequest::Out(transfer_out(
            TsUrbOutKind::CtlTransfer(urb),
            UrbFunction::URB_FUNCTION_CONTROL_TRANSFER,
            data,
        )),
    }
}

fn validate_control_data(setup: SetupPacket, data: &[u8]) -> Result<(), ConversionError> {
    match setup.request_type.direction() {
        Direction::In if !data.is_empty() => Err(ConversionError::InTransferHasData { actual: data.len() }),
        Direction::Out if data.len() != usize::from(setup.length) => Err(ConversionError::OutTransferLengthMismatch {
            expected: usize::from(setup.length),
            actual: data.len(),
        }),
        Direction::In | Direction::Out => Ok(()),
    }
}

#[derive(Debug, Clone, Copy)]
enum DescriptorOperation {
    Get,
    Set,
}

fn descriptor_function(recipient: Recipient, operation: DescriptorOperation) -> Option<UrbFunction> {
    match (operation, recipient) {
        (DescriptorOperation::Get, Recipient::DEVICE) => Some(UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE),
        (DescriptorOperation::Get, Recipient::INTERFACE) => {
            Some(UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE)
        }
        (DescriptorOperation::Get, Recipient::ENDPOINT) => Some(UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT),
        (DescriptorOperation::Set, Recipient::DEVICE) => Some(UrbFunction::URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE),
        (DescriptorOperation::Set, Recipient::INTERFACE) => Some(UrbFunction::URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE),
        (DescriptorOperation::Set, Recipient::ENDPOINT) => Some(UrbFunction::URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT),
        _ => None,
    }
}

fn feature_function(request: u8, recipient: Recipient) -> Option<UrbFunction> {
    match (request, recipient) {
        (standard_request::CLEAR_FEATURE, Recipient::DEVICE) => Some(UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_DEVICE),
        (standard_request::CLEAR_FEATURE, Recipient::INTERFACE) => {
            Some(UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_INTERFACE)
        }
        (standard_request::CLEAR_FEATURE, Recipient::ENDPOINT) => {
            Some(UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT)
        }
        (standard_request::CLEAR_FEATURE, Recipient::OTHER) => Some(UrbFunction::URB_FUNCTION_CLEAR_FEATURE_TO_OTHER),
        (standard_request::SET_FEATURE, Recipient::DEVICE) => Some(UrbFunction::URB_FUNCTION_SET_FEATURE_TO_DEVICE),
        (standard_request::SET_FEATURE, Recipient::INTERFACE) => {
            Some(UrbFunction::URB_FUNCTION_SET_FEATURE_TO_INTERFACE)
        }
        (standard_request::SET_FEATURE, Recipient::ENDPOINT) => Some(UrbFunction::URB_FUNCTION_SET_FEATURE_TO_ENDPOINT),
        (standard_request::SET_FEATURE, Recipient::OTHER) => Some(UrbFunction::URB_FUNCTION_SET_FEATURE_TO_OTHER),
        _ => None,
    }
}

fn get_status_function(recipient: Recipient) -> Option<UrbFunction> {
    match recipient {
        Recipient::DEVICE => Some(UrbFunction::URB_FUNCTION_GET_STATUS_FROM_DEVICE),
        Recipient::INTERFACE => Some(UrbFunction::URB_FUNCTION_GET_STATUS_FROM_INTERFACE),
        Recipient::ENDPOINT => Some(UrbFunction::URB_FUNCTION_GET_STATUS_FROM_ENDPOINT),
        Recipient::OTHER => Some(UrbFunction::URB_FUNCTION_GET_STATUS_FROM_OTHER),
        _ => None,
    }
}

fn vendor_or_class_function(kind: RequestKind, recipient: Recipient) -> Option<UrbFunction> {
    match (kind, recipient) {
        (RequestKind::VENDOR, Recipient::DEVICE) => Some(UrbFunction::URB_FUNCTION_VENDOR_DEVICE),
        (RequestKind::VENDOR, Recipient::INTERFACE) => Some(UrbFunction::URB_FUNCTION_VENDOR_INTERFACE),
        (RequestKind::VENDOR, Recipient::ENDPOINT) => Some(UrbFunction::URB_FUNCTION_VENDOR_ENDPOINT),
        (RequestKind::VENDOR, Recipient::OTHER) => Some(UrbFunction::URB_FUNCTION_VENDOR_OTHER),
        (RequestKind::CLASS, Recipient::DEVICE) => Some(UrbFunction::URB_FUNCTION_CLASS_DEVICE),
        (RequestKind::CLASS, Recipient::INTERFACE) => Some(UrbFunction::URB_FUNCTION_CLASS_INTERFACE),
        (RequestKind::CLASS, Recipient::ENDPOINT) => Some(UrbFunction::URB_FUNCTION_CLASS_ENDPOINT),
        (RequestKind::CLASS, Recipient::OTHER) => Some(UrbFunction::URB_FUNCTION_CLASS_OTHER),
        _ => None,
    }
}

fn configuration_descriptor(descriptor: ConfigurationDescriptorSet<'_>) -> Result<UsbConfigDesc, ConversionError> {
    let configuration = descriptor.configuration();
    if usize::from(configuration.length()) != UsbConfigDesc::FIXED_PART_SIZE {
        return Err(ConversionError::InvalidConfigurationHeaderLength {
            actual: configuration.length(),
        });
    }

    Ok(UsbConfigDesc {
        length: configuration.length(),
        descriptor_type: configuration.raw_descriptor().descriptor_type(),
        total_length: configuration.total_length(),
        num_interfaces: configuration.num_interfaces(),
        configuration_value: configuration.configuration_value(),
        configuration: configuration.configuration_string(),
        attributes: configuration.attributes().raw(),
        max_power: configuration.max_power_raw(),
        trailing: descriptor.as_bytes()[UsbConfigDesc::FIXED_PART_SIZE..].to_vec(),
    })
}

fn interface_information(interface: InterfaceDescriptor<'_>) -> Result<TsUsbdInterfaceInfo, ConversionError> {
    let selection = InterfaceSelection {
        interface: interface.number(),
        alternate_setting: interface.alternate_setting(),
    };
    let mut pipes = Vec::with_capacity(interface.endpoints().count());
    for endpoint in interface.endpoints() {
        let raw_max_packet_size = endpoint.max_packet_size().raw();
        if interface.alternate_setting() == 0
            && endpoint.transfer_type() == TransferType::Isochronous
            && raw_max_packet_size != 0
        {
            return Err(ConversionError::InvalidMaximumPacketSize {
                selection,
                raw: raw_max_packet_size,
            });
        }
        pipes.push(TsUsbdPipeInfo {
            // The client opens the pipe and reports the packet size it actually
            // established in TS_USBD_PIPE_INFORMATION_RESULT, so the request
            // field only states a preference. Windows servers leave it zero,
            // and a server-side value would have to be derived from the
            // announced device speed, which USB_DEVICE_CAPABILITIES
            // ([MS-RDPEUSB] 2.2.11) cannot express beyond full and high.
            max_packet_size: 0,
            // MS-RDPEUSB 2.2.9.1.3 notes that the client ignores this
            // obsolete USBD field. Zero requests the client default.
            max_transfer_size: 0,
            // No general USB semantic requests a Windows pipe override.
            pipe_flags: 0,
        });
    }

    Ok(TsUsbdInterfaceInfo {
        interface_number: selection.interface,
        alternate_setting: selection.alternate_setting,
        ts_usbd_pipe_info: pipes,
    })
}

fn in_transfer_flags(direction: Direction) -> u32 {
    match direction {
        Direction::In => USBD_TRANSFER_DIRECTION_IN | USBD_SHORT_TRANSFER_OK,
        Direction::Out => 0,
    }
}

fn completion_status(hresult: u32, usbd_status: u32) -> UsbResult<()> {
    if !hresult_succeeded(hresult) {
        return Err(UsbError::Error);
    }

    usb_status(usbd_status)
}

fn hresult_succeeded(hresult: u32) -> bool {
    hresult & HRESULT_FAILURE_BIT == 0
}

fn usb_status(usbd_status: u32) -> UsbResult<()> {
    match usbd_status {
        USBD_STATUS_SUCCESS => Ok(()),
        USBD_STATUS_CANCELED => Err(UsbError::Cancelled),
        USBD_STATUS_STALL_PID => Err(UsbError::Stall),
        USBD_STATUS_TIMEOUT => Err(UsbError::Timeout),
        USBD_STATUS_BABBLE_DETECTED => Err(UsbError::Overflow),
        USBD_STATUS_DEVICE_GONE => Err(UsbError::NoDevice),
        _ => Err(UsbError::Error),
    }
}

fn packet_status_raw(status: i32) -> u32 {
    u32::from_ne_bytes(status.to_ne_bytes())
}

fn header_only_transfer_in(completion: CompletionData) -> Result<TransferInCompletionResult, CompletionError> {
    let CompletionData::TransferIn(completion) = completion else {
        return Err(CompletionError::ExpectedTransferIn);
    };
    if completion.ts_urb_result.payload.is_some() {
        return Err(CompletionError::UnexpectedUrbResultPayload);
    }
    Ok(completion)
}

fn ensure_empty_output(completion: &TransferInCompletionResult) -> Result<(), CompletionError> {
    if completion.output_buffer.is_empty() {
        Ok(())
    } else {
        Err(CompletionError::UnexpectedOutputData {
            actual: completion.output_buffer.len(),
        })
    }
}

fn byte_transfer_in_completion(completion: CompletionData) -> Result<UsbResult<u8>, CompletionError> {
    let completion = header_only_transfer_in(completion)?;
    let status = completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status);
    if let Err(error) = status {
        return Ok(Err(error));
    }
    if completion.output_buffer.len() != 1 {
        return Err(CompletionError::ExpectedOneByteOutput {
            actual: completion.output_buffer.len(),
        });
    }

    Ok(Ok(completion.output_buffer[0]))
}

fn unit_transfer_in_completion(completion: CompletionData) -> Result<UsbResult<()>, CompletionError> {
    let completion = header_only_transfer_in(completion)?;
    ensure_empty_output(&completion)?;
    let status = completion_status(completion.hresult, completion.ts_urb_result.header.usbd_status);
    Ok(status)
}

fn rdpeusb_setup_packet(setup: SetupPacket) -> RdpeusbSetupPacket {
    RdpeusbSetupPacket {
        request_type: setup.request_type.raw(),
        request: setup.request,
        value: setup.value,
        index: setup.index,
        length: setup.length,
    }
}

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
