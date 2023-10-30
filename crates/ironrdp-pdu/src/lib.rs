#![allow(clippy::arithmetic_side_effects)] // FIXME: remove
#![allow(clippy::cast_lossless)] // FIXME: remove
#![allow(clippy::cast_possible_truncation)] // FIXME: remove
#![allow(clippy::cast_possible_wrap)] // FIXME: remove
#![allow(clippy::cast_sign_loss)] // FIXME: remove

use core::fmt;

use cursor::WriteCursor;
use write_buf::WriteBuf;

use crate::cursor::ReadCursor;

#[macro_use]
mod macros;

pub mod codecs;
pub mod cursor;
pub mod gcc;
pub mod geometry;
pub mod input;
pub mod mcs;
pub mod nego;
pub mod padding;
pub mod pcb;
pub mod rdp;
pub mod tpdu;
pub mod tpkt;
pub mod utf16;
pub mod utils;
pub mod write_buf;
pub mod x224;

pub(crate) mod basic_output;
pub(crate) mod ber;
pub(crate) mod crypto;
pub(crate) mod per;

pub use crate::basic_output::{bitmap, fast_path, pointer, surface_commands};
pub use crate::rdp::vc::dvc;

// FIXME: remove
pub use ironrdp_common::{
    assert_impl, assert_obj_safe, encode, encode_buf, encode_cursor, size, Decode as PduDecode, Encode as PduEncode,
    IntoOwned as IntoOwnedPdu, Pdu, PduError, PduErrorExt, PduErrorKind, PduResult,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Action {
    FastPath = 0x00,
    X224 = 0x03,
}

impl Action {
    pub fn from_fp_output_header(fp_output_header: u8) -> Result<Self, u8> {
        match fp_output_header & 0b11 {
            0x00 => Ok(Self::FastPath),
            0x03 => Ok(Self::X224),
            unknown_action_bits => Err(unknown_action_bits),
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PduInfo {
    pub action: Action,
    pub length: usize,
}

/// Finds next RDP PDU size by reading the next few bytes.
pub fn find_size(bytes: &[u8]) -> PduResult<Option<PduInfo>> {
    macro_rules! ensure_enough {
        ($bytes:expr, $len:expr) => {
            if $bytes.len() < $len {
                return Ok(None);
            }
        };
    }

    ensure_enough!(bytes, 1);
    let fp_output_header = bytes[0];

    let action = Action::from_fp_output_header(fp_output_header)
        .map_err(|unknown_action| PduError::unexpected_message_type("fpOutputHeader", unknown_action))?;

    match action {
        Action::X224 => {
            ensure_enough!(bytes, crate::tpkt::TpktHeader::SIZE);
            let tpkt = crate::tpkt::TpktHeader::read(&mut ReadCursor::new(bytes))?;

            Ok(Some(PduInfo {
                action,
                length: tpkt.packet_length(),
            }))
        }
        Action::FastPath => {
            ensure_enough!(bytes, 2);
            let a = bytes[1];

            let fast_path_length = if a & 0x80 != 0 {
                ensure_enough!(bytes, 3);
                let b = bytes[2];

                ((u16::from(a) & !0x80) << 8) + u16::from(b)
            } else {
                u16::from(a)
            };

            Ok(Some(PduInfo {
                action,
                length: usize::from(fast_path_length),
            }))
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct X224Hint;

pub const X224_HINT: X224Hint = X224Hint;

impl PduHint for X224Hint {
    fn find_size(&self, bytes: &[u8]) -> PduResult<Option<usize>> {
        match find_size(bytes)? {
            Some(pdu_info) => {
                debug_assert_eq!(pdu_info.action, Action::X224);
                Ok(Some(pdu_info.length))
            }
            None => Ok(None),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FastPathHint;

pub const FAST_PATH_HINT: FastPathHint = FastPathHint;

impl PduHint for FastPathHint {
    fn find_size(&self, bytes: &[u8]) -> PduResult<Option<usize>> {
        match find_size(bytes)? {
            Some(pdu_info) => {
                debug_assert_eq!(pdu_info.action, Action::FastPath);
                Ok(Some(pdu_info.length))
            }
            None => Ok(None),
        }
    }
}

pub use legacy::*;

// TODO: Delete these traits at some point
mod legacy {
    use thiserror::Error;

    pub trait PduParsing {
        type Error;

        fn from_buffer(stream: impl std::io::Read) -> Result<Self, Self::Error>
        where
            Self: Sized;
        fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error>;
        fn buffer_length(&self) -> usize;
    }

    /// Blanket implementation for references to types implementing PduParsing. Only encoding is supported.
    ///
    /// This helps removing a few copies.
    impl<T: PduParsing> PduParsing for &T {
        type Error = T::Error;

        fn from_buffer(_: impl std::io::Read) -> Result<Self, Self::Error>
        where
            Self: Sized,
        {
            panic!("Can’t return a reference to a local value")
        }

        fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error> {
            T::to_buffer(self, stream)
        }

        fn buffer_length(&self) -> usize {
            T::buffer_length(self)
        }
    }

    pub trait PduBufferParsing<'a>: Sized {
        type Error;

        fn from_buffer(mut buffer: &'a [u8]) -> Result<Self, Self::Error> {
            Self::from_buffer_consume(&mut buffer)
        }
        fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error>;
        fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error>;
        fn buffer_length(&self) -> usize;
    }

    #[derive(Debug, Error)]
    pub enum RdpError {
        #[error("IO error")]
        IOError(#[from] std::io::Error),
        #[error("Received invalid action code: {0}")]
        InvalidActionCode(u8),
    }
}
