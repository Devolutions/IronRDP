use core::fmt;

pub type PduResult<T> = Result<T, PduError>;

pub type PduError = ironrdp_error::Error<PduErrorKind>;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum PduErrorKind {
    NotEnoughBytes { received: usize, expected: usize },
    InvalidMessage { field: &'static str, reason: &'static str },
    UnexpectedMessageType { got: u8 },
    UnsupportedVersion { got: u8 },
    Other { description: &'static str },
    Custom,
}

#[cfg(feature = "std")]
impl std::error::Error for PduErrorKind {}

impl fmt::Display for PduErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughBytes { received, expected } => write!(
                f,
                "not enough bytes provided to decode: received {received} bytes, expected {expected} bytes"
            ),
            Self::InvalidMessage { field, reason } => {
                write!(f, "invalid `{field}`: {reason}")
            }
            Self::UnexpectedMessageType { got } => {
                write!(f, "invalid message type ({got})")
            }
            Self::UnsupportedVersion { got } => {
                write!(f, "unsupported version ({got})")
            }
            Self::Other { description } => {
                write!(f, "{description}")
            }
            Self::Custom => {
                write!(f, "custom error")
            }
        }
    }
}

pub trait PduErrorExt {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self;
    fn invalid_message(context: &'static str, field: &'static str, reason: &'static str) -> Self;
    fn unexpected_message_type(context: &'static str, got: u8) -> Self;
    fn unsupported_version(context: &'static str, got: u8) -> Self;
    fn other(context: &'static str, description: &'static str) -> Self;
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: ironrdp_error::Source;
}

impl PduErrorExt for PduError {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self {
        Self::new(context, PduErrorKind::NotEnoughBytes { received, expected })
    }

    fn invalid_message(context: &'static str, field: &'static str, reason: &'static str) -> Self {
        Self::new(context, PduErrorKind::InvalidMessage { field, reason })
    }

    fn unexpected_message_type(context: &'static str, got: u8) -> Self {
        Self::new(context, PduErrorKind::UnexpectedMessageType { got })
    }

    fn unsupported_version(context: &'static str, got: u8) -> Self {
        Self::new(context, PduErrorKind::UnsupportedVersion { got })
    }

    fn other(context: &'static str, description: &'static str) -> Self {
        Self::new(context, PduErrorKind::Other { description })
    }

    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: ironrdp_error::Source,
    {
        Self::new(context, PduErrorKind::Custom).with_source(e)
    }
}
