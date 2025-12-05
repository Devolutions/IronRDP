#![allow(clippy::return_self_not_must_use)]
use core::fmt::Display;

use ironrdp::cliprdr::backend::ClipboardError;
use ironrdp::connector::ConnectorError;
use ironrdp::session::SessionError;
#[cfg(target_os = "windows")]
use ironrdp_cliprdr_native::WinCliprdrError;
use ironrdp_rdcleanpath::der;

use self::ffi::IronRdpErrorKind;

pub struct GenericError(pub anyhow::Error);

impl Display for GenericError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

struct IronRdpErrorInner {
    repr: String,
    kind: IronRdpErrorKind,
}

// Helper function to create an IronRdpError
fn make_ffi_error(repr: String, kind: IronRdpErrorKind) -> Box<ffi::IronRdpError> {
    Box::new(ffi::IronRdpError(IronRdpErrorInner { repr, kind }))
}

// Direct conversion from IronRdpErrorKind (for cases with no underlying error)
impl From<IronRdpErrorKind> for Box<ffi::IronRdpError> {
    fn from(kind: IronRdpErrorKind) -> Self {
        make_ffi_error(kind.to_string(), kind)
    }
}

// IronRDP errors - use .report() to include full error chain with sources
impl From<ConnectorError> for Box<ffi::IronRdpError> {
    fn from(value: ConnectorError) -> Self {
        let kind = match value.kind() {
            ironrdp::connector::ConnectorErrorKind::Encode(_) => IronRdpErrorKind::EncodeError,
            ironrdp::connector::ConnectorErrorKind::Decode(_) => IronRdpErrorKind::DecodeError,
            ironrdp::connector::ConnectorErrorKind::Credssp(_) => IronRdpErrorKind::CredsspError,
            ironrdp::connector::ConnectorErrorKind::AccessDenied => IronRdpErrorKind::AccessDenied,
            _ => IronRdpErrorKind::Generic,
        };
        let repr = value.report().to_string();
        make_ffi_error(repr, kind)
    }
}

impl From<SessionError> for Box<ffi::IronRdpError> {
    fn from(value: SessionError) -> Self {
        let kind = match value.kind() {
            ironrdp::session::SessionErrorKind::Pdu(_) => IronRdpErrorKind::PduError,
            ironrdp::session::SessionErrorKind::Encode(_) => IronRdpErrorKind::EncodeError,
            ironrdp::session::SessionErrorKind::Decode(_) => IronRdpErrorKind::DecodeError,
            _ => IronRdpErrorKind::Generic,
        };
        let repr = value.report().to_string();
        make_ffi_error(repr, kind)
    }
}

impl From<ironrdp::pdu::PduError> for Box<ffi::IronRdpError> {
    fn from(value: ironrdp::pdu::PduError) -> Self {
        let repr = value.report().to_string();
        make_ffi_error(repr, IronRdpErrorKind::PduError)
    }
}

impl From<ironrdp::core::EncodeError> for Box<ffi::IronRdpError> {
    fn from(value: ironrdp::core::EncodeError) -> Self {
        let repr = value.report().to_string();
        make_ffi_error(repr, IronRdpErrorKind::EncodeError)
    }
}

impl From<ironrdp::core::DecodeError> for Box<ffi::IronRdpError> {
    fn from(value: ironrdp::core::DecodeError) -> Self {
        let repr = value.report().to_string();
        make_ffi_error(repr, IronRdpErrorKind::DecodeError)
    }
}

// std::io::Error - convert to anyhow::Error for proper source chain formatting
impl From<std::io::Error> for Box<ffi::IronRdpError> {
    fn from(value: std::io::Error) -> Self {
        let repr = format!("{:#}", anyhow::Error::new(value));
        make_ffi_error(repr, IronRdpErrorKind::IO)
    }
}

// sspi::Error - convert to anyhow::Error for proper source chain formatting
impl From<sspi::Error> for Box<ffi::IronRdpError> {
    fn from(value: sspi::Error) -> Self {
        let repr = format!("{:#}", anyhow::Error::new(value));
        make_ffi_error(repr, IronRdpErrorKind::CredsspError)
    }
}

// Simple string error
impl From<&str> for Box<ffi::IronRdpError> {
    fn from(value: &str) -> Self {
        make_ffi_error(value.to_string(), IronRdpErrorKind::Generic)
    }
}

// core::fmt::Error - convert to anyhow::Error for consistency
impl From<core::fmt::Error> for Box<ffi::IronRdpError> {
    fn from(value: core::fmt::Error) -> Self {
        let repr = format!("{:#}", anyhow::Error::new(value));
        make_ffi_error(repr, IronRdpErrorKind::Generic)
    }
}

// Clipboard errors - manually format with full source chain
impl From<&dyn ClipboardError> for Box<ffi::IronRdpError> {
    fn from(value: &dyn ClipboardError) -> Self {
        use std::fmt::Write as _;

        // Manually build error chain since we have a trait object reference
        let mut repr = value.to_string();
        let mut source = value.source();
        while let Some(e) = source {
            let _ = write!(&mut repr, ", caused by: {}", e);
            source = e.source();
        }
        make_ffi_error(repr, IronRdpErrorKind::Clipboard)
    }
}

#[cfg(target_os = "windows")]
impl From<WinCliprdrError> for Box<ffi::IronRdpError> {
    fn from(value: WinCliprdrError) -> Self {
        let repr = format!("{:#}", anyhow::Error::new(value));
        make_ffi_error(repr, IronRdpErrorKind::Clipboard)
    }
}

// DER errors - convert to anyhow::Error for proper source chain formatting
impl From<der::Error> for Box<ffi::IronRdpError> {
    fn from(value: der::Error) -> Self {
        let repr = format!("{:#}", anyhow::Error::new(value));
        make_ffi_error(repr, IronRdpErrorKind::DecodeError)
    }
}

impl From<ironrdp_rdcleanpath::MissingRDCleanPathField> for Box<ffi::IronRdpError> {
    fn from(value: ironrdp_rdcleanpath::MissingRDCleanPathField) -> Self {
        let repr = format!("{:#}", anyhow::Error::new(value));
        make_ffi_error(repr, IronRdpErrorKind::Generic)
    }
}

// GenericError already has proper Display impl with {:#}
impl From<GenericError> for Box<ffi::IronRdpError> {
    fn from(value: GenericError) -> Self {
        make_ffi_error(value.to_string(), IronRdpErrorKind::Generic)
    }
}

// FFI-specific errors
impl From<ValueConsumedError> for Box<ffi::IronRdpError> {
    fn from(value: ValueConsumedError) -> Self {
        make_ffi_error(value.to_string(), IronRdpErrorKind::Consumed)
    }
}

impl From<IncorrectEnumTypeError> for Box<ffi::IronRdpError> {
    fn from(value: IncorrectEnumTypeError) -> Self {
        make_ffi_error(value.to_string(), IronRdpErrorKind::IncorrectEnumType)
    }
}

impl From<WrongOSError> for Box<ffi::IronRdpError> {
    fn from(value: WrongOSError) -> Self {
        make_ffi_error(value.to_string(), IronRdpErrorKind::WrongOS)
    }
}

#[diplomat::bridge]
pub mod ffi {
    use core::fmt::Write as _;

    use diplomat_runtime::DiplomatWriteable;

    #[derive(Debug, Clone, Copy, thiserror::Error)]
    pub enum IronRdpErrorKind {
        #[error("Generic error")]
        Generic,
        #[error("PDU error")]
        PduError,
        #[error("Encode error")]
        EncodeError,
        #[error("Decode error")]
        DecodeError,
        #[error("CredSSP error")]
        CredsspError,
        #[error("Value is consumed")]
        Consumed,
        #[error("IO error")]
        IO,
        #[error("Access denied")]
        AccessDenied,
        #[error("Incorrect rust enum type")]
        IncorrectEnumType,
        #[error("Clipboard error")]
        Clipboard,
        #[error("wrong platform error")]
        WrongOS,
    }

    /// Stringified Picky error along with an error kind.
    #[diplomat::opaque]
    pub struct IronRdpError(pub(super) super::IronRdpErrorInner);

    impl IronRdpError {
        /// Returns the error as a string.
        pub fn to_display(&self, writeable: &mut DiplomatWriteable) {
            let _ = write!(writeable, "{}", self.0.repr);
            writeable.flush();
        }

        /// Returns the error kind.
        pub fn get_kind(&self) -> IronRdpErrorKind {
            self.0.kind
        }
    }
}

#[derive(Debug)]
pub struct ValueConsumedError {
    item: String,
    reason: Option<String>,
}

impl ValueConsumedError {
    pub fn for_item(item: &str) -> ValueConsumedError {
        ValueConsumedError {
            item: item.to_owned(),
            reason: None,
        }
    }

    pub fn reason(mut self, reason: &str) -> ValueConsumedError {
        self.reason = Some(reason.to_owned());
        self
    }
}

impl Display for ValueConsumedError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(reason) = &self.reason {
            write!(f, "{}: {}", self.item, reason)
        } else {
            write!(f, "{}: is consumed or never constructed", self.item)
        }
    }
}

impl From<ValueConsumedError> for IronRdpErrorKind {
    fn from(_val: ValueConsumedError) -> Self {
        IronRdpErrorKind::Consumed
    }
}

pub struct IncorrectEnumTypeError {
    expected: &'static str,
    enum_name: &'static str,
}

impl IncorrectEnumTypeError {
    pub fn on_variant(variant: &'static str) -> IncorrectEnumTypeErrorBuilder {
        IncorrectEnumTypeErrorBuilder { expected: variant }
    }
}

pub struct IncorrectEnumTypeErrorBuilder {
    expected: &'static str,
}

impl IncorrectEnumTypeErrorBuilder {
    pub fn of_enum(self, enum_name: &'static str) -> IncorrectEnumTypeError {
        IncorrectEnumTypeError {
            expected: self.expected,
            enum_name,
        }
    }
}

impl Display for IncorrectEnumTypeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "expected enum variable {} of enum {}", self.expected, self.enum_name)
    }
}

impl From<IncorrectEnumTypeError> for IronRdpErrorKind {
    fn from(_val: IncorrectEnumTypeError) -> Self {
        IronRdpErrorKind::IncorrectEnumType
    }
}

pub struct WrongOSError {
    expected: &'static str,
    custom_message: Option<String>,
}

impl WrongOSError {
    pub fn expected_platform(expected: &'static str) -> WrongOSError {
        WrongOSError {
            expected,
            custom_message: None,
        }
    }

    pub fn with_custom_message(mut self, message: &str) -> WrongOSError {
        self.custom_message = Some(message.to_owned());
        self
    }
}

impl Display for WrongOSError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(custom_message) = &self.custom_message {
            write!(f, "{custom_message}")?;
        }
        write!(f, "expected platform {}", self.expected)
    }
}
