//! Messages specific to the [USB Device][1] interface.
//!
//! This interface is used by the client to communicate with the server about new USB devices. Has
//! no default ID, is alloted an interface ID during the lifetime of a USB Redirection Channel.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/034257d7-f7a8-4fe1-b8c2-87ac8dc4f50e

pub type RequestId = u32;

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size};
use ironrdp_pdu::utils::strict_sum;

use crate::pdu::common::{Interface, SharedMsgHeader};

/// The `CANCEL_REQUEST` message is sent from the server to the client to cancel an outstanding IO
/// request.
///
/// * [MS-RDPEUSB § 2.2.6.1 Cancel Request Message (CANCEL_REQUEST)][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/93912b05-1fc8-4a43-8abd-78d9aab65d71
#[doc(alias = "CANCEL_REQUEST")]
pub struct CancelRequest {
    /// The `InterfaceId` field **MUST** match the value sent previously in the `UsbDevice` field
    /// of the [`ADD_DEVICE`][1] message. The `Mask` field **MUST** be set to
    /// [`STREAM_ID_PROXY`][2]. The `FunctionId` field **MUST** be set to [`CANCEL_REQUEST`][3].
    ///
    /// [1]: crate::pdu::dev_sink::AddDevice
    /// [2]: crate::pdu::common::Mask::StreamIdProxy
    /// [3]: crate::pdu::common::FunctionId::CANCEL_REQUEST
    pub header: SharedMsgHeader,
    /// Request ID of the oustanding IO request to cancel previously sent via [`IO_CONTROL`],
    /// [`INTERNAL_IO_CONTROL`], [`TRANSFER_IN_REQUEST`], or [`TRANSFER_OUT_REQUEST`] message.
    pub req_id: RequestId,
}

impl Encode for CancelRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header.encode(dst)?;
        dst.write_u32(self.req_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "CANCEL_REQUEST"
    }

    fn size(&self) -> usize {
        let header = self.header.size();
        const REQ_ID: usize = const { size_of::<RequestId>() };

        strict_sum(&[header + REQ_ID])
    }
}

impl Decode<'_> for CancelRequest {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;
        ensure_size!(in: src, size: size_of::<RequestId>());
        let req_id = src.read_u32();

        Ok(Self { header, req_id })
    }
}

/// The `REGISTER_REQUEST_CALLBACK` message is sent from the server to the client to provide an
/// interface ID for the **Request Completion** interface to the client.
///
/// * [MS-RDPEUSB § 2.2.6.2 Register Request Callback Message (REGISTER_REQUEST_CALLBACK)][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/8693de72-5e87-4b64-a252-101e865311a5
#[doc(alias = "REGISTER_REQUEST_CALLBACK")]
pub struct RegisterRequestCallback {
    /// The `InterfaceId` field **MUST** match the value sent previously in the `UsbDevice` field
    /// of the [`ADD_DEVICE`][1] message. The `Mask` field **MUST** be set to
    /// [`STREAM_ID_PROXY`][2]. The `FunctionId` field **MUST** be set to
    /// [`REGISTER_REQUEST_CALLBACK`][3].
    ///
    /// [1]: crate::pdu::dev_sink::AddDevice
    /// [2]: crate::pdu::common::Mask::StreamIdProxy
    /// [3]: crate::pdu::common::FunctionId::REGISTER_REQUEST_CALLBACK
    pub header: SharedMsgHeader,
    /// A unique `InterfaceID` to be used by all messages defined in the **Request Completion**
    /// interface.
    ///
    /// NOTE: `Interface` **MUST** be the [`NonDefault`][1] variant.
    ///
    /// [1]: crate::pdu::common::Interface::NonDefault
    pub request_completion: Option<Interface>,
}

impl Encode for RegisterRequestCallback {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        if let Some(request_completion) = self.request_completion {
            dst.write_u32(0x1);
            dst.write_u32(request_completion.into());
        } else {
            dst.write_u32(0x0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "REGISTER_REQUEST_CALLBACK"
    }

    fn size(&self) -> usize {
        let header = self.header.size();
        const NUM_REQUEST_COMPLETION: usize = const { size_of::<u32>() };
        let request_completion = if self.request_completion.is_some() {
            size_of_val(&self.request_completion)
        } else {
            0
        };

        strict_sum(&[header + NUM_REQUEST_COMPLETION + request_completion])
    }
}

impl Decode<'_> for RegisterRequestCallback {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;
        ensure_size!(in: src, size: size_of::<u32>());
        let request_completion = match src.read_u32(/* NumRequestCompletion */) {
            0 => None,
            id => Some(Interface::from(id)),
        };
        Ok(Self {
            header,
            request_completion,
        })
    }
}
