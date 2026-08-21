//! Messages specific to the [Request Completion][1] interface.
//!
//! Used by the client to send the final result for a request previously sent from the server.
//! The unique interface ID for this interface is provided by the server using the
//! [`RegisterRequestCallback`] message, during the lifecycle of a USB redirection channel.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c0a146fc-20cf-4897-af27-a3c5474151ac

use alloc::vec::Vec;

use ironrdp_core::{
    Decode as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err, other_err,
};
use ironrdp_dvc::DvcEncode;
use ironrdp_pdu::utils::strict_sum;

use crate::pdu::completion::ts_urb_result::{Raw, TsUrbIsochTransferResult, TsUrbResult, TsUrbResultPayload};
use crate::pdu::header::{FunctionId, InterfaceId, Mask, MessageId, SharedMsgHeader};
#[cfg(doc)]
use crate::pdu::usb_dev::{
    InternalIoControl, IoControl, RegisterRequestCallback, TransferInRequest, TransferOutRequest,
};
use crate::pdu::utils::{HResult, RequestIdIoctl, RequestIdTransferInOut};

/// * [MS-ERREF § 2.2 Win32 Error Codes][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/18d8fbe8-a967-4f1c-ae50-99ca8e491d2d
const ERROR_INSUFFICIENT_BUFFER: u32 = 0x7A;

/// * [MS-ERREF § 2.1.2 HRESULT From WIN32 Error Code Macro][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/0c0bcf55-277e-4120-b5dc-f6115fc8dc38
const FACILITY_WIN32: u32 = 0x7;

pub mod ts_urb_result;

/// * [MS-ERREF § 2.1.2 HRESULT From WIN32 Error Code Macro][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/0c0bcf55-277e-4120-b5dc-f6115fc8dc38
macro_rules! HRESULT_FROM_WIN32 {
    ($x: expr) => {{
        if $x & 0x80000000 != 0 || $x == 0 {
            $x
        } else {
            $x & 0x0000FFFF | (FACILITY_WIN32 << 16) | 0x80000000
        }
    }};
}

const HRESULT_FROM_WIN32_ERROR_INSUFFICIENT_BUFFER: u32 = HRESULT_FROM_WIN32!(ERROR_INSUFFICIENT_BUFFER);

/// [\[MS-RDPEUSB\] 2.2.7.1 IO Control Completion (IOCONTROL_COMPLETION)][1] packet.
///
/// Sent from the client to the server as the final result of an [`IoControl`] or
/// [`InternalIoControl`] request.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b1722374-0658-47ba-8368-87bf9d3db4d4
#[doc(alias = "IOCONTROL_COMPLETION")]
#[derive(Debug, PartialEq, Clone)]
pub struct IoControlCompletion {
    pub msg_id: MessageId,
    /// The interface ID provided by the server in the `RequestCompletion` field of the prior
    /// [`RegisterRequestCallback`] message.
    pub completion_iface: InterfaceId,
    pub request_id: RequestIdIoctl,
    pub hresult: HResult,
    pub information: u32,
    pub output_buffer_size: u32,
    pub output_buffer: Vec<u8>,
}

impl IoControlCompletion {
    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.completion_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::IOCONTROL_COMPLETION),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        const FIXED: usize = 4 /* RequestId */ + 4 /* HResult */ + 4 /* Information */ + 4 /* OutputBufferSize */;
        ensure_size!(in: src, size: FIXED);

        let request_id = src.read_u32();
        let hresult = src.read_u32();
        let information = src.read_u32();
        let output_buffer_size = src.read_u32();

        let n = output_buffer_size.try_into().map_err(|e| other_err!(source: e))?;

        let output_buffer = match hresult {
            0 => {
                if information != output_buffer_size {
                    return Err(invalid_field_err!(
                        "Information != OutputBufferSize",
                        "HResult is: 0x0 (IOCTL success), but Information != OutputBufferSize"
                    ));
                }
                ensure_size!(in: src, size: n);
                src.read_slice(n).to_vec()
            }
            HRESULT_FROM_WIN32_ERROR_INSUFFICIENT_BUFFER => {
                ensure_size!(in: src, size: n);
                src.read_slice(n).to_vec()
            }
            _ => {
                if output_buffer_size != 0 {
                    // > If the HResult field is equal to HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)
                    // > then ... . For any other case `OutputBufferSize` **MUST** be set to 0 ...
                    //
                    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b1722374-0658-47ba-8368-87bf9d3db4d4
                    return Err(invalid_field_err!(
                        "OutputBufferSize",
                        "HResult is not one of: 0x0 (success), 0x8007007A (insufficient buffer error), \
                    so expected OutputBufferSize: 0x0"
                    ));
                }
                Vec::new()
            }
        };

        Ok(Self {
            msg_id,
            completion_iface: udev_iface,
            request_id,
            hresult,
            information,
            output_buffer_size,
            output_buffer,
        })
    }
}

impl Encode for IoControlCompletion {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header().encode(dst)?;

        dst.write_u32(self.request_id);
        dst.write_u32(self.hresult);
        dst.write_u32(self.information);
        dst.write_u32(self.output_buffer_size);

        dst.write_slice(&self.output_buffer);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "IOCONTROL_COMPLETION"
    }

    fn size(&self) -> usize {
        strict_sum(&[SharedMsgHeader::SIZE_REQ
            + const {
                size_of::<RequestIdIoctl>(/* RequestId */)
                    + size_of::<HResult>()
                    + 4 /* Information */
                    + 4 /* OutputBufferSize */
            }
            + self.output_buffer.len()])
    }
}

impl DvcEncode for IoControlCompletion {}

fn decode_raw_ts_urb_result(src: &mut ReadCursor<'_>, size: usize) -> DecodeResult<TsUrbResult<Raw>> {
    ensure_size!(in: src, size: size);
    let mut result_src = ReadCursor::new(src.read_slice(size));
    let result = TsUrbResult::decode(&mut result_src)?;
    if !result_src.is_empty() {
        return Err(invalid_field_err!(
            "CbTsUrbResult",
            "does not match TS_URB_RESULT_HEADER::Size"
        ));
    }

    Ok(result)
}

/// [\[MS-RDPEUSB\] 2.2.7.2 URB Completion (URB_COMPLETION)][1] packet.
///
/// Sent from the client to the server as the final result of a [`TransferInRequest`] that contains
/// output data.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/5bfa9c84-a74b-4942-9d09-e770b21081eb
#[doc(alias = "URB_COMPLETION")]
#[derive(Debug, PartialEq, Clone)]
pub struct UrbCompletion {
    pub msg_id: MessageId,
    /// The interface ID provided by the server in the `RequestCompletion` field of the prior
    /// [`RegisterRequestCallback`] message.
    pub completion_iface: InterfaceId,
    pub req_id: RequestIdTransferInOut,
    pub ts_urb_result: TsUrbResult<TsUrbIsochTransferResult>,
    pub hresult: HResult,
    pub output_buffer: Vec<u8>,
}

impl UrbCompletion {
    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.completion_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::URB_COMPLETION),
        }
    }
    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* RequestId */ + 4 /* CbTsUrbResult */);
        let req_id = RequestIdTransferInOut::try_from(src.read_u32())?;

        let cb_ts_urb_result: usize = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
        let ts_urb_result = decode_raw_ts_urb_result(src, cb_ts_urb_result)?.into_isoch()?;

        ensure_size!(in: src, size: 4 /* HResult */ + 4 /* OutputBufferSize */);
        let hresult = src.read_u32();
        let output_buffer_size = usize::try_from(src.read_u32()).map_err(|e| other_err!(source: e))?;
        ensure_size!(in: src, size: output_buffer_size);
        let output_buffer = src.read_slice(output_buffer_size).to_vec();
        Ok(Self {
            msg_id,
            completion_iface: udev_iface,
            req_id,
            ts_urb_result,
            hresult,
            output_buffer,
        })
    }
}

impl Encode for UrbCompletion {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header().encode(dst)?;
        dst.write_u32(self.req_id.into());
        match u32::try_from(self.ts_urb_result.size()) {
            Ok(cb_ts_urb_result) => dst.write_u32(cb_ts_urb_result),
            Err(e) => return Err(other_err!(source: e)),
        }

        self.ts_urb_result.encode(dst)?;
        dst.write_u32(self.hresult);
        dst.write_u32(self.output_buffer.len().try_into().map_err(|e| other_err!(source: e))?);
        dst.write_slice(&self.output_buffer);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "URB_COMPLETION"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_REQ
            + size_of::<u32>(/* RequestId */)
            + size_of::<u32>(/* CbTsUrbResult */)
            + self.ts_urb_result.size()
            + size_of::<u32>(/* HResult */)
            + size_of::<u32>(/* OutputBufferSize */)
            + self.output_buffer.len()
    }
}

impl DvcEncode for UrbCompletion {}

/// [\[MS-RDPEUSB\] 2.2.7.3 URB Completion No Data (URB_COMPLETION_NO_DATA)][1] packet.
///
/// Sent from the client to the server as the final result of a [`TransferInRequest`] that contains
/// no output data or a [`TransferOutRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/994fac8f-d258-47a6-aa35-48783abe49ec
#[doc(alias = "URB_COMPLETION_NO_DATA")]
#[derive(Debug, PartialEq, Clone)]
pub struct UrbCompletionNoData<P> {
    pub msg_id: MessageId,
    /// The interface ID provided by the server in the `RequestCompletion` field of the prior
    /// [`RegisterRequestCallback`] message.
    pub completion_iface: InterfaceId,
    pub req_id: RequestIdTransferInOut,
    pub ts_urb_result: TsUrbResult<P>,
    pub hresult: HResult,
    pub output_buffer_size: u32,
}

impl<T> UrbCompletionNoData<T> {
    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.completion_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::URB_COMPLETION_NO_DATA),
        }
    }
}

impl UrbCompletionNoData<Raw> {
    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* RequestId */ + 4 /* CbTsUrbResult */);
        let req_id = RequestIdTransferInOut::try_from(src.read_u32())?;

        let cb_ts_urb_result = usize::try_from(src.read_u32()).map_err(|e| other_err!(source: e))?;
        let ts_urb_result = decode_raw_ts_urb_result(src, cb_ts_urb_result)?;
        ensure_size!(in: src, size: 4 /* HResult */ + 4 /* OutputBufferSize */);
        let hresult = src.read_u32();
        let output_buffer_size = src.read_u32();
        Ok(Self {
            msg_id,
            completion_iface: udev_iface,
            req_id,
            ts_urb_result,
            hresult,
            output_buffer_size,
        })
    }
}

impl Encode for UrbCompletionNoData<TsUrbResultPayload> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header().encode(dst)?;
        dst.write_u32(self.req_id.into());
        match self.ts_urb_result.size().try_into() {
            Ok(cb_ts_urb_result) => dst.write_u32(cb_ts_urb_result),
            Err(e) => return Err(other_err!(source: e)),
        }
        self.ts_urb_result.encode(dst)?;
        dst.write_u32(self.hresult);
        dst.write_u32(self.output_buffer_size);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "URB_COMPLETION_NO_DATA"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_REQ
            + size_of::<u32>(/* RequestId */)
            + size_of::<u32>(/* CbTsUrbResult */)
            + self.ts_urb_result.size()
            + size_of::<u32>(/* HResult */)
            + size_of::<u32>(/* OutputBufferSize */)
    }
}

impl DvcEncode for UrbCompletionNoData<TsUrbResultPayload> {}
