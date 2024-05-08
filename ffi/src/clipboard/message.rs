#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct ClipboardMessage(pub ironrdp::cliprdr::backend::ClipboardMessage);

    impl ClipboardMessage {
        pub fn get_enum_type(&self) -> ClipboardMessageType {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendInitiateCopy(_) => {
                    ClipboardMessageType::SendInitiateCopy
                }
                ironrdp::cliprdr::backend::ClipboardMessage::SendFormatData(_) => ClipboardMessageType::SendFormatData,
                ironrdp::cliprdr::backend::ClipboardMessage::SendInitiatePaste(_) => {
                    ClipboardMessageType::SendInitiatePaste
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

        pub fn get_send_format_data(&self) -> Option<Box<OwndFormatDataResponse>> {
            match &self.0 {
                ironrdp::cliprdr::backend::ClipboardMessage::SendFormatData(val) => Some(val.clone()),
                _ => None,
            }
            .map(Some)
            .map(OwndFormatDataResponse)
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
    }

    pub enum ClipboardMessageType {
        SendInitiateCopy,
        SendFormatData,
        SendInitiatePaste,
        Error,
    }

    #[diplomat::opaque]
    pub struct ClipboardFormatIterator(pub Vec<ironrdp::cliprdr::pdu::ClipboardFormat>);

    #[diplomat::opaque]
    pub struct OwndFormatDataResponse(pub Option<ironrdp::cliprdr::pdu::OwnedFormatDataResponse>);

    #[diplomat::opaque]
    pub struct ClipboardFormatId(pub ironrdp::cliprdr::pdu::ClipboardFormatId);
}
