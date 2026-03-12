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
use ironrdp_pdu::utils::strict_sum;

use crate::pdu::completion::ts_urb_result::{TsUrbIsochTransferResult, TsUrbResult, TsUrbResultPayload};
use crate::pdu::header::SharedMsgHeader;
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
    pub header: SharedMsgHeader,
    pub request_id: RequestIdIoctl,
    pub hresult: HResult,
    pub information: u32,
    pub output_buffer_size: u32,
    pub output_buffer: Vec<u8>,
}

impl IoControlCompletion {
    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        const FIXED: usize = size_of::<RequestIdIoctl>() + size_of::<HResult>() + size_of::<u32>() + size_of::<u32>();
        ensure_size!(in: src, size: FIXED);

        let request_id = src.read_u32();
        let hresult = src.read_u32();
        let information = src.read_u32();
        let output_buffer_size = src.read_u32();

        // Should this stuff be part of some validate() function?
        if hresult == 0 {
            if information != output_buffer_size {
                return Err(invalid_field_err!(
                    "Information != OutputBufferSize",
                    "HResult is: 0x0 (IOCTL success), but Information != OutputBufferSize"
                ));
            }
        } else if hresult != HRESULT_FROM_WIN32_ERROR_INSUFFICIENT_BUFFER && output_buffer_size != 0 {
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

        let output_buffer = match hresult {
            // #[expect(clippy::as_conversions)]
            0 | HRESULT_FROM_WIN32_ERROR_INSUFFICIENT_BUFFER => {
                let n = information.try_into().map_err(|e| other_err!(source: e))?;
                Vec::from(src.read_slice(n))
            }
            // > For any other case [OutputBufferSize] MUST be set to 0
            // Which means empty output_buffer
            _ => Vec::new(),
        };

        Ok(Self {
            header,
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

        self.header.encode(dst)?;

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
        #[expect(clippy::as_conversions)]
        let out_buf = if self.hresult == 0 {
            assert_eq!(self.information, self.output_buffer_size);
            self.output_buffer.len()
        } else if self.hresult == HRESULT_FROM_WIN32_ERROR_INSUFFICIENT_BUFFER {
            self.information as usize
        } else {
            0
        };

        strict_sum(&[SharedMsgHeader::SIZE_REQ
            + const {
                size_of::<RequestIdIoctl>(/* RequestId */)
                    + size_of::<HResult>()
                    + size_of::<u32>(/* Information */)
                    + size_of::<u32>(/* OutputBufferSize */)
            }
            + out_buf])
    }
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
    pub header: SharedMsgHeader,
    pub req_id: RequestIdTransferInOut,
    pub ts_urb_result: TsUrbResult,
    pub hresult: HResult,
    pub output_buffer: Vec<u8>,
}

impl UrbCompletion {
    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<u32>(/* RequestId */) + size_of::<u32>(/* CbTsUrbResult */));
        let req_id = RequestIdTransferInOut::try_from(src.read_u32())
            .map_err(|reason| invalid_field_err!("URB_COMPLETION::RequestId", reason))?;

        let cb_ts_urb_result: usize = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
        ensure_size!(in: src, size: cb_ts_urb_result);
        let mut ts_urb_result = TsUrbResult::decode(&mut ReadCursor::new(src.read_slice(cb_ts_urb_result)))?;
        let TsUrbResultPayload::Raw(bytes) = ts_urb_result.payload else {
            unreachable!("TsUrbResultPayload::decode always returns Raw(_)")
        };
        ts_urb_result.payload = if bytes.is_empty() {
            TsUrbResultPayload::Raw(bytes)
        } else {
            // URB_COMPLETION's TsUrbResult can only have a payload iff it's isoch
            TsUrbResultPayload::Isoch(TsUrbIsochTransferResult::decode(&mut ReadCursor::new(&bytes))?)
        };

        ensure_size!(in: src, size: size_of::<u32>(/* HResult */) + size_of::<u32>(/* OutputBufferSize */));
        let hresult = src.read_u32();
        let output_buffer_size = usize::try_from(src.read_u32()).map_err(|e| other_err!(source: e))?;
        ensure_size!(in: src, size: output_buffer_size);
        let output_buffer = src.read_slice(output_buffer_size).to_vec();
        Ok(Self {
            header,
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
        self.header.encode(dst)?;
        dst.write_u32(self.req_id.into());
        match u32::try_from(self.ts_urb_result.size()) {
            Ok(cb_ts_urb_result) => dst.write_u32(cb_ts_urb_result),
            Err(e) => return Err(other_err!(source: e)),
        }
        if !matches!(self.ts_urb_result.payload, TsUrbResultPayload::Isoch(_))
            && self.ts_urb_result.payload != TsUrbResultPayload::Raw(Vec::new())
        {
            return Err(invalid_field_err!(
                "URB_COMPLETION::TsUrbResult",
                "has non-empty payload but payload is not TS_URB_ISOCH_TRANSFER_RESULT"
            ));
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

/// [\[MS-RDPEUSB\] 2.2.7.3 URB Completion No Data (URB_COMPLETION_NO_DATA)][1] packet.
///
/// Sent from the client to the server as the final result of a [`TransferInRequest`] that contains
/// no output data or a [`TransferOutRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/994fac8f-d258-47a6-aa35-48783abe49ec
#[doc(alias = "URB_COMPLETION_NO_DATA")]
#[derive(Debug, PartialEq, Clone)]
pub struct UrbCompletionNoData {
    pub header: SharedMsgHeader,
    pub req_id: RequestIdTransferInOut,
    pub ts_urb_result: TsUrbResult,
    pub hresult: HResult,
    pub output_buffer_size: u32,
}

impl UrbCompletionNoData {
    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<u32>(/* RequestId */) + size_of::<u32>(/* CbTsUrbResult */));
        let req_id = RequestIdTransferInOut::try_from(src.read_u32())
            .map_err(|reason| invalid_field_err!("URB_COMPLETION::RequestId", reason))?;

        let cb_ts_urb_result = usize::try_from(src.read_u32()).map_err(|e| other_err!(source: e))?;
        ensure_size!(in: src, size: cb_ts_urb_result);
        let ts_urb_result = TsUrbResult::decode(&mut ReadCursor::new(src.read_slice(cb_ts_urb_result)))?;
        ensure_size!(in: src, size: size_of::<u32>(/* HResult */) + size_of::<u32>(/* OutputBufferSize */));
        let hresult = src.read_u32();
        let output_buffer_size = src.read_u32();
        Ok(Self {
            header,
            req_id,
            ts_urb_result,
            hresult,
            output_buffer_size,
        })
    }
}

impl Encode for UrbCompletionNoData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
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

#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::vec;

    use ts_urb_result::TsUrbResultHeader;

    use super::*;
    use crate::pdu::completion::ts_urb_result::TsUrbGetCurrFrameNumResult;
    use crate::pdu::header::{FunctionId, InterfaceId};
    use crate::pdu::utils::{UsbdIsoPacketDesc, round_trip};

    #[test]
    fn iocontrol_completion() {
        let mut en = IoControlCompletion {
            header: SharedMsgHeader {
                interface_id: InterfaceId(25),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 14,
                function_id: Some(FunctionId::IOCONTROL_COMPLETION),
            },
            request_id: 876,
            hresult: 0,
            information: 4,
            output_buffer_size: 4,
            output_buffer: vec![1, 2, 3, 4],
        };
        let de = round_trip!(en, IoControlCompletion);
        assert_eq!(en, de);

        en.hresult = HRESULT_FROM_WIN32_ERROR_INSUFFICIENT_BUFFER;
        en.output_buffer_size = 2;
        en.information = 2;
        en.output_buffer = vec![1, 2];
        let de = round_trip!(en, IoControlCompletion);
        assert_eq!(en, de);

        en.hresult = 13124;
        en.information = 89374;
        en.output_buffer_size = 0;
        en.output_buffer.clear();
        let de = round_trip!(en, IoControlCompletion);
        assert_eq!(en, de);
    }

    #[test]
    fn urb_completion() {
        let mut en = UrbCompletion {
            header: SharedMsgHeader {
                interface_id: InterfaceId(78435),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 234,
                function_id: Some(FunctionId::URB_COMPLETION),
            },
            req_id: RequestIdTransferInOut::try_from(234).unwrap(),
            ts_urb_result: TsUrbResult {
                header: TsUrbResultHeader { usbd_status: 0 },
                payload: TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
                    start_frame: 123,
                    error_count: 1,
                    iso_packet: vec![
                        UsbdIsoPacketDesc {
                            offset: 0,
                            length: 2,
                            status: 0,
                        },
                        UsbdIsoPacketDesc {
                            offset: 2,
                            length: 2,
                            status: -1,
                        },
                        UsbdIsoPacketDesc {
                            offset: 4,
                            length: 2,
                            status: 0,
                        },
                    ],
                }),
            },
            hresult: HRESULT_FROM_WIN32!(0u32),
            output_buffer: vec![1, 2, 3, 4, 5, 6],
        };

        let de = round_trip!(en, UrbCompletion);
        let mut buf = vec![0; en.ts_urb_result.payload.size()];
        en.ts_urb_result
            .payload
            .encode(&mut WriteCursor::new(&mut buf))
            .unwrap();
        assert_eq!(en, de);

        en.ts_urb_result.payload = TsUrbResultPayload::Raw(vec![]); // IO_CONTROL / INTERNAL_IO_CONTROL
        let de = round_trip!(en, UrbCompletion);
        assert_eq!(en, de);
    }

    #[test]
    fn urb_completion_no_data() {
        let mut en = UrbCompletionNoData {
            header: SharedMsgHeader {
                interface_id: InterfaceId(78435),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 234,
                function_id: Some(FunctionId::URB_COMPLETION_NO_DATA),
            },
            req_id: RequestIdTransferInOut::try_from(234).unwrap(),
            ts_urb_result: TsUrbResult {
                header: TsUrbResultHeader { usbd_status: 0 },
                payload: TsUrbResultPayload::FrameNum(TsUrbGetCurrFrameNumResult { frame_number: 234 }),
            },
            hresult: HRESULT_FROM_WIN32!(0u32),
            output_buffer_size: 0,
        };

        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let mut src = ReadCursor::new(&buf);
        let de = SharedMsgHeader::decode(&mut src)
            .and_then(|header| <UrbCompletionNoData>::decode(&mut src, header))
            .unwrap();

        let mut buf = vec![0; en.ts_urb_result.payload.size()];
        en.ts_urb_result
            .payload
            .encode(&mut WriteCursor::new(&mut buf))
            .unwrap();
        en.ts_urb_result.payload = TsUrbResultPayload::Raw(buf);
        assert_eq!(en, de);
    }
}
