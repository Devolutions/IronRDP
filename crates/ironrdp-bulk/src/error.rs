//! Error types for bulk compression operations.

use core::fmt;

/// Error type for bulk compression and decompression operations.
#[derive(Debug)]
pub enum BulkError {
    /// The compression type value is not supported.
    UnsupportedCompressionType(u32),
    /// The compressed data is malformed or truncated.
    InvalidCompressedData(&'static str),
    /// The output buffer is too small for the decompressed data.
    OutputBufferTooSmall {
        /// Required minimum size.
        required: usize,
        /// Actual available size.
        available: usize,
    },
    /// The history buffer overflowed.
    HistoryBufferOverflow,
    /// A decompression operation encountered an unexpected end of input.
    UnexpectedEndOfInput,
}

impl fmt::Display for BulkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCompressionType(value) => {
                write!(f, "unsupported compression type: {value:#04x}")
            }
            Self::InvalidCompressedData(detail) => {
                write!(f, "invalid compressed data: {detail}")
            }
            Self::OutputBufferTooSmall { required, available } => {
                write!(
                    f,
                    "output buffer too small: need {required} bytes, but only {available} available"
                )
            }
            Self::HistoryBufferOverflow => {
                write!(f, "history buffer overflow")
            }
            Self::UnexpectedEndOfInput => {
                write!(f, "unexpected end of input")
            }
        }
    }
}

impl core::error::Error for BulkError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::UnsupportedCompressionType(_) => None,
            Self::InvalidCompressedData(_) => None,
            Self::OutputBufferTooSmall { .. } => None,
            Self::HistoryBufferOverflow => None,
            Self::UnexpectedEndOfInput => None,
        }
    }
}
