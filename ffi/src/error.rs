use std::fmt;

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

    /// Kind associated to a Picky Error
    #[derive(Clone, Copy)]
    pub enum IronRdpErrorKind {
        /// Generic Picky error
        Generic,
        PduError,
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
