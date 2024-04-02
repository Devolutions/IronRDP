#![allow(clippy::return_self_not_must_use)]
use std::fmt::Display;

use ironrdp::connector::ConnectorError;

use self::ffi::IronRdpErrorKind;

impl From<ConnectorError> for IronRdpErrorKind {
    fn from(val: ConnectorError) -> Self {
        match val.kind {
            ironrdp::connector::ConnectorErrorKind::Pdu(_) => IronRdpErrorKind::PduError,
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

impl From<std::io::Error> for IronRdpErrorKind {
    fn from(_: std::io::Error) -> Self {
        IronRdpErrorKind::IO
    }
}

impl From<std::fmt::Error> for IronRdpErrorKind {
    fn from(_val: std::fmt::Error) -> Self {
        IronRdpErrorKind::Generic
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
    use diplomat_runtime::DiplomatWriteable;
    use std::fmt::Write as _;

    #[derive(Debug, Clone, Copy, thiserror::Error)]
    pub enum IronRdpErrorKind {
        #[error("Generic error")]
        Generic,
        #[error("PDU error")]
        PduError,
        #[error("CredSSP error")]
        CredsspError,
        #[error("Value is consumed")]
        Consumed,
        #[error("IO error")]
        IO,
        #[error("Access denied")]
        AccessDenied,
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
