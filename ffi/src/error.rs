#![allow(clippy::return_self_not_must_use)]
use core::fmt::Display;

use ironrdp::cliprdr::backend::ClipboardError;
use ironrdp::connector::ConnectorError;
use ironrdp::session::SessionError;
#[cfg(target_os = "windows")]
use ironrdp_cliprdr_native::WinCliprdrError;

use self::ffi::IronRdpErrorKind;

impl From<ConnectorError> for IronRdpErrorKind {
    fn from(val: ConnectorError) -> Self {
        match val.kind {
            ironrdp::connector::ConnectorErrorKind::Encode(_) => IronRdpErrorKind::EncodeError,
            ironrdp::connector::ConnectorErrorKind::Decode(_) => IronRdpErrorKind::DecodeError,
            ironrdp::connector::ConnectorErrorKind::Credssp(_) => IronRdpErrorKind::CredsspError,
            ironrdp::connector::ConnectorErrorKind::AccessDenied => IronRdpErrorKind::AccessDenied,
            _ => IronRdpErrorKind::Generic,
        }
    }
}

impl From<&str> for IronRdpErrorKind {
    fn from(_val: &str) -> Self {
        IronRdpErrorKind::Generic
    }
}

impl From<sspi::Error> for IronRdpErrorKind {
    fn from(_val: sspi::Error) -> Self {
        IronRdpErrorKind::CredsspError
    }
}

impl From<ironrdp::pdu::PduError> for IronRdpErrorKind {
    fn from(_val: ironrdp::pdu::PduError) -> Self {
        IronRdpErrorKind::PduError
    }
}

impl From<ironrdp::core::EncodeError> for IronRdpErrorKind {
    fn from(_val: ironrdp::core::EncodeError) -> Self {
        IronRdpErrorKind::EncodeError
    }
}

impl From<ironrdp::core::DecodeError> for IronRdpErrorKind {
    fn from(_val: ironrdp::core::DecodeError) -> Self {
        IronRdpErrorKind::DecodeError
    }
}

impl From<std::io::Error> for IronRdpErrorKind {
    fn from(_: std::io::Error) -> Self {
        IronRdpErrorKind::IO
    }
}

impl From<core::fmt::Error> for IronRdpErrorKind {
    fn from(_val: core::fmt::Error) -> Self {
        IronRdpErrorKind::Generic
    }
}

impl From<SessionError> for IronRdpErrorKind {
    fn from(value: SessionError) -> Self {
        match value.kind() {
            ironrdp::session::SessionErrorKind::Pdu(_) => IronRdpErrorKind::PduError,
            ironrdp::session::SessionErrorKind::Encode(_) => IronRdpErrorKind::EncodeError,
            ironrdp::session::SessionErrorKind::Decode(_) => IronRdpErrorKind::DecodeError,
            _ => IronRdpErrorKind::Generic,
        }
    }
}

impl From<&dyn ClipboardError> for IronRdpErrorKind {
    fn from(_val: &dyn ClipboardError) -> Self {
        IronRdpErrorKind::Clipboard
    }
}

#[cfg(target_os = "windows")]
impl From<WinCliprdrError> for IronRdpErrorKind {
    fn from(_val: WinCliprdrError) -> Self {
        IronRdpErrorKind::Clipboard
    }
}

impl From<WrongOSError> for IronRdpErrorKind {
    fn from(_val: WrongOSError) -> Self {
        IronRdpErrorKind::WrongOS
    }
}

impl<T> From<T> for Box<ffi::IronRdpError>
where
    T: Into<IronRdpErrorKind> + ToString,
{
    fn from(value: T) -> Self {
        let repr = value.to_string();
        let kind = value.into();
        Box::new(ffi::IronRdpError(IronRdpErrorInner { repr, kind }))
    }
}

struct IronRdpErrorInner {
    repr: String,
    kind: IronRdpErrorKind,
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
        #[error("Missing required field")]
        MissingRequiredField,
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
        write!(
            f,
            "expected enum variable {}, of enum {}",
            self.expected, self.enum_name
        )
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
