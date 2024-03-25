use ironrdp::connector::ConnectorError;

use self::ffi::IronRdpErrorKind;

impl Into<ffi::IronRdpErrorKind> for ConnectorError {
    fn into(self) -> ffi::IronRdpErrorKind {
        match self.kind {
            ironrdp::connector::ConnectorErrorKind::Pdu(_) => todo!(),
            ironrdp::connector::ConnectorErrorKind::Credssp(_) => todo!(),
            ironrdp::connector::ConnectorErrorKind::Reason(_) => todo!(),
            ironrdp::connector::ConnectorErrorKind::AccessDenied => todo!(),
            ironrdp::connector::ConnectorErrorKind::General => todo!(),
            ironrdp::connector::ConnectorErrorKind::Custom => todo!(),
            _ => todo!(),
        }
    }
}

impl Into<ffi::IronRdpErrorKind> for &str {
    fn into(self) -> ffi::IronRdpErrorKind {
        ffi::IronRdpErrorKind::Generic
    }
}

impl Into<ffi::IronRdpErrorKind> for ironrdp::pdu::PduError {
    fn into(self) -> ffi::IronRdpErrorKind {
        match self {
            _ => ffi::IronRdpErrorKind::PduError,
        }
    }
}

impl Into<ffi::IronRdpErrorKind> for std::io::Error {
    fn into(self) -> ffi::IronRdpErrorKind {
        match self.kind() {
            _ => ffi::IronRdpErrorKind::IO,
        }
    }
}

impl Into<ffi::IronRdpErrorKind> for std::fmt::Error {
    fn into(self) -> ffi::IronRdpErrorKind {
        ffi::IronRdpErrorKind::Generic
    }
}

impl<T> From<T> for Box<ffi::IronRdpError>
where
    T: Into<ffi::IronRdpErrorKind> + ToString,
{
    fn from(value: T) -> Self {
        let repr = value.to_string();
        let kind = value.into();
        Box::new(ffi::IronRdpError(IronRdpErrorInner { repr, kind }))
    }
}

struct IronRdpErrorInner {
    pub repr: String,
    pub kind: ffi::IronRdpErrorKind,
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
        SspiError,
        #[error("Null pointer error")]
        NullPointer,
        #[error("IO error")]
        IO,
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

        /// Prints the error string.
        pub fn print(&self) {
            println!("{}", self.0.repr);
        }

        /// Returns the error kind.
        pub fn get_kind(&self) -> IronRdpErrorKind {
            self.0.kind
        }
    }
}


#[derive(Debug)]
pub struct NullPointerError{
    item: String,
    reason: Option<String>,
}

impl NullPointerError {
    pub fn for_item(item: &str) -> NullPointerError {
        NullPointerError {
            item: item.to_string(),
            reason: None,
        }
    }

    pub fn reason(mut self, reason: &str) -> NullPointerError {
        self.reason = Some(reason.to_string());
        self
    }
}

impl ToString for NullPointerError {
    fn to_string(&self) -> String {
        format!("{}: {:?}", self.item, self.reason)
    }
}

impl Into<IronRdpErrorKind> for NullPointerError {
    fn into(self) -> IronRdpErrorKind {
        IronRdpErrorKind::NullPointer
    }
}