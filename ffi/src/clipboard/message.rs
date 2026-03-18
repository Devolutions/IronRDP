#[diplomat::bridge]
pub mod ffi {

    use crate::error::ffi::IronRdpError;
    use crate::utils::ffi::VecU8;

    #[diplomat::opaque]
    pub struct ClipboardMessage(pub ironrdp::cliprdr::backend::ClipboardMessage);

    impl ClipboardMessage {
        pub fn get_message_type(&self) -> ClipboardMessageType {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendInitiateCopy(_) => {
                    ClipboardMessageType::SendInitiateCopy
                }
                ironrdp::cliprdr::backend::ClipboardMessage::SendFormatData(_) => ClipboardMessageType::SendFormatData,
                ironrdp::cliprdr::backend::ClipboardMessage::SendInitiatePaste(_) => {
                    ClipboardMessageType::SendInitiatePaste
                }
                ironrdp::cliprdr::backend::ClipboardMessage::SendFileContentsRequest(_) => {
                    ClipboardMessageType::SendFileContentsRequest
                }
                ironrdp::cliprdr::backend::ClipboardMessage::SendFileContentsResponse(_) => {
                    ClipboardMessageType::SendFileContentsResponse
                }
                ironrdp::cliprdr::backend::ClipboardMessage::Error(_) => ClipboardMessageType::Error,
            }
        }

        pub fn get_send_initiate_copy(&self) -> Option<Box<ClipboardFormatIterator>> {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendInitiateCopy(val) => Some(val.clone()),
                _ => None,
            }
            .map(ClipboardFormatIterator)
            .map(Box::new)
        }

        pub fn get_send_format_data(&self) -> Option<Box<FormatDataResponse>> {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendFormatData(val) => Some(val.clone()),
                _ => None,
            }
            .map(Some)
            .map(FormatDataResponse)
            .map(Box::new)
        }

        pub fn get_send_initiate_paste(&self) -> Option<Box<ClipboardFormatId>> {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendInitiatePaste(val) => Some(*val),
                _ => None,
            }
            .map(ClipboardFormatId)
            .map(Box::new)
        }

        pub fn get_send_file_contents_request(&self) -> Option<Box<FfiFileContentsRequest>> {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendFileContentsRequest(val) => Some(val.clone()),
                _ => None,
            }
            .map(FfiFileContentsRequest)
            .map(Box::new)
        }

        pub fn get_send_file_contents_response(&self) -> Option<Box<FfiFileContentsResponse>> {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendFileContentsResponse(val) => Some(val.clone()),
                _ => None,
            }
            .map(FfiFileContentsResponse)
            .map(Box::new)
        }

        pub fn get_error(&self) -> Option<Box<IronRdpError>> {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::Error(e) => {
                    let error_ref: &dyn ironrdp::cliprdr::backend::ClipboardError = e.as_ref();
                    Some(error_ref.into())
                }
                _ => None,
            }
        }
    }

    pub enum ClipboardMessageType {
        SendInitiateCopy,
        SendFormatData,
        SendInitiatePaste,
        SendFileContentsRequest,
        SendFileContentsResponse,
        Error,
    }

    #[diplomat::opaque]
    pub struct ClipboardFormatIterator(pub Vec<ironrdp::cliprdr::pdu::ClipboardFormat>);

    #[diplomat::opaque]
    pub struct FormatDataResponse(pub Option<ironrdp::cliprdr::pdu::OwnedFormatDataResponse>);

    #[diplomat::opaque]
    pub struct ClipboardFormatId(pub ironrdp::cliprdr::pdu::ClipboardFormatId);

    #[diplomat::opaque]
    pub struct FfiFileContentsRequest(pub ironrdp::cliprdr::pdu::FileContentsRequest);

    impl FfiFileContentsRequest {
        pub fn stream_id(&self) -> u32 {
            self.0.stream_id
        }

        pub fn index(&self) -> i32 {
            self.0.index
        }

        pub fn is_size_request(&self) -> bool {
            self.0.flags.contains(ironrdp::cliprdr::pdu::FileContentsFlags::SIZE)
        }

        pub fn is_range_request(&self) -> bool {
            self.0.flags.contains(ironrdp::cliprdr::pdu::FileContentsFlags::RANGE)
        }

        pub fn position(&self) -> u64 {
            self.0.position
        }

        pub fn requested_size(&self) -> u32 {
            self.0.requested_size
        }

        pub fn has_data_id(&self) -> bool {
            self.0.data_id.is_some()
        }

        pub fn data_id(&self) -> Result<u32, Box<IronRdpError>> {
            self.0.data_id.ok_or_else(|| "no data_id present in request".into())
        }
    }

    /// Wraps `OwnedFileContentsResponse`, which is a type alias for
    /// `FileContentsResponse<'static>` (generated by `impl_pdu_borrowing!`).
    #[diplomat::opaque]
    pub struct FfiFileContentsResponse(pub ironrdp::cliprdr::pdu::OwnedFileContentsResponse);

    impl FfiFileContentsResponse {
        pub fn stream_id(&self) -> u32 {
            self.0.stream_id()
        }

        pub fn is_error(&self) -> bool {
            self.0.is_error()
        }

        pub fn data(&self) -> Box<VecU8> {
            Box::new(VecU8(self.0.data().to_vec()))
        }
    }
}
