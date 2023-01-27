#[cfg(test)]
mod test;

use std::io::{self, Write};

use bit_field::BitField;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use thiserror::Error;

use super::bitmap::{Bitmap, BitmapError};
use super::surface_commands::{SurfaceCommand, SurfaceCommandsError, SURFACE_COMMAND_HEADER_SIZE};
use crate::utils::SplitTo;
use crate::{per, PduBufferParsing, PduParsing};

/// Implements the Fast-Path RDP message header PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathHeader {
    pub flags: EncryptionFlags,
    pub data_length: usize,
    forced_long_length: bool,
}

impl FastPathHeader {
    pub fn from_buffer_with_header(mut stream: impl io::Read, header: u8) -> Result<Self, FastPathError> {
        let flags = EncryptionFlags::from_bits_truncate(header.get_bits(6..8));

        let (length, sizeof_length) = per::read_length(&mut stream)?;
        if length < sizeof_length as u16 + 1 {
            return Err(FastPathError::NullLength {
                bytes_read: sizeof_length + 1,
            });
        }
        let data_length = length as usize - sizeof_length - 1;
        // Detect case, when received packet has non-optimal packet length packing
        let forced_long_length = per::sizeof_length(length) != sizeof_length;

        Ok(FastPathHeader {
            flags,
            data_length,
            forced_long_length,
        })
    }

    fn minimal_buffer_length(&self) -> usize {
        1 + per::sizeof_length(self.data_length as u16)
    }
}

impl PduParsing for FastPathHeader {
    type Error = FastPathError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let header = stream.read_u8()?;

        Self::from_buffer_with_header(&mut stream, header)
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let mut header = 0u8;
        header.set_bits(0..2, 0); // fast-path action
        header.set_bits(6..8, self.flags.bits());
        stream.write_u8(header)?;

        if self.forced_long_length {
            // Preserve same layout for header as received
            per::write_long_length(stream, (self.data_length + self.buffer_length()) as u16)?;
        } else {
            per::write_length(stream, (self.data_length + self.minimal_buffer_length()) as u16)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        if self.forced_long_length {
            1 + per::SIZEOF_U16
        } else {
            self.minimal_buffer_length()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathUpdatePdu<'a> {
    pub fragmentation: Fragmentation,
    pub update_code: UpdateCode,
    pub data: &'a [u8],
}

impl<'a> PduBufferParsing<'a> for FastPathUpdatePdu<'a> {
    type Error = FastPathError;

    fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let header = buffer.read_u8()?;

        let update_code = header.get_bits(0..4);
        let update_code = UpdateCode::from_u8(update_code).ok_or(FastPathError::InvalidUpdateCode(update_code))?;

        let fragmentation = header.get_bits(4..6);
        let fragmentation =
            Fragmentation::from_u8(fragmentation).ok_or(FastPathError::InvalidFragmentation(fragmentation))?;

        let compression = Compression::from_bits_truncate(header.get_bits(6..8));
        if compression.contains(Compression::COMPRESSION_USED) {
            return Err(FastPathError::CompressionNotSupported);
        }

        let data_length = usize::from(buffer.read_u16::<LittleEndian>()?);
        if buffer.len() < data_length {
            return Err(FastPathError::InvalidDataLength {
                expected: data_length,
                actual: buffer.len(),
            });
        }
        let data = buffer.split_to(data_length);

        Ok(Self {
            fragmentation,
            update_code,
            data,
        })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        let mut header = 0u8;
        header.set_bits(0..4, self.update_code.to_u8().unwrap());
        header.set_bits(4..6, self.fragmentation.to_u8().unwrap());
        buffer.write_u8(header)?;
        buffer.write_u16::<LittleEndian>(self.data.len() as u16)?;
        buffer.write_all(self.data)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        3 + self.data.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastPathUpdate<'a> {
    SurfaceCommands(Vec<SurfaceCommand<'a>>),
    Bitmap(Bitmap<'a>),
}

impl<'a> FastPathUpdate<'a> {
    pub fn from_buffer_with_code(mut buffer: &'a [u8], code: UpdateCode) -> Result<Self, FastPathError> {
        Self::from_buffer_consume_with_code(&mut buffer, code)
    }

    pub fn from_buffer_consume_with_code(buffer: &mut &'a [u8], code: UpdateCode) -> Result<Self, FastPathError> {
        match code {
            UpdateCode::SurfaceCommands => {
                let mut commands = Vec::with_capacity(1);
                while buffer.len() >= SURFACE_COMMAND_HEADER_SIZE {
                    commands.push(SurfaceCommand::from_buffer_consume(buffer)?);
                }

                Ok(Self::SurfaceCommands(commands))
            }
            UpdateCode::Bitmap => {
                let bitmap = Bitmap::from_buffer_consume(buffer).map_err(FastPathError::BitmapError)?;
                Ok(Self::Bitmap(bitmap))
            }
            _ => Err(FastPathError::UnsupportedFastPathUpdate(code)),
        }
    }

    pub fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), FastPathError> {
        match self {
            Self::SurfaceCommands(ref commands) => {
                for command in commands {
                    command.to_buffer_consume(buffer)?;
                }
            }
            Self::Bitmap(ref bitmap) => {
                bitmap.to_buffer_consume(buffer)?;
            }
        }

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        match self {
            Self::SurfaceCommands(commands) => commands.iter().map(|c| c.buffer_length()).sum::<usize>(),
            Self::Bitmap(bitmap) => bitmap.buffer_length(),
        }
    }

    pub fn as_short_name(&self) -> &str {
        match self {
            Self::SurfaceCommands(_) => "Surface Commands",
            Self::Bitmap(_) => "Bitmap",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum UpdateCode {
    Orders = 0x0,
    Bitmap = 0x1,
    Palette = 0x2,
    Synchronize = 0x3,
    SurfaceCommands = 0x4,
    HiddenPointer = 0x5,
    DefaultPointer = 0x6,
    PositionPointer = 0x8,
    ColorPointer = 0x9,
    CachedPointer = 0xa,
    NewPointer = 0xb,
    LargePointer = 0xc,
}

impl<'a> From<&FastPathUpdate<'a>> for UpdateCode {
    fn from(update: &FastPathUpdate<'_>) -> Self {
        match update {
            FastPathUpdate::SurfaceCommands(_) => Self::SurfaceCommands,
            FastPathUpdate::Bitmap(_) => Self::Bitmap,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum Fragmentation {
    Single = 0x0,
    Last = 0x1,
    First = 0x2,
    Next = 0x3,
}

bitflags! {
    pub struct EncryptionFlags: u8 {
        const SECURE_CHECKSUM = 0x1;
        const ENCRYPTED = 0x2;
    }
}

bitflags! {
    pub struct Compression: u8 {
        const COMPRESSION_USED = 0x2;
    }
}

/// The type of a Fast-Path parsing error. Includes *length error* and *I/O error*.
#[derive(Debug, Error)]
pub enum FastPathError {
    /// May be used in I/O related errors such as receiving empty Fast-Path packages.
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("Surface Commands error")]
    SurfaceCommandsError(#[from] SurfaceCommandsError),
    #[error("Bitmap error: {0}")]
    BitmapError(#[from] BitmapError),
    /// Used in the length-related error during Fast-Path parsing.
    #[error("Received invalid Fast-Path package with 0 length")]
    NullLength { bytes_read: usize },
    #[error("Received invalid update code: {0}")]
    InvalidUpdateCode(u8),
    #[error("Received invalid fragmentation: {0}")]
    InvalidFragmentation(u8),
    #[error("Received compressed Fast-Path package")]
    CompressionNotSupported,
    #[error("Input buffer is shorter then the data length: {} < {}", actual, expected)]
    InvalidDataLength { expected: usize, actual: usize },
    #[error("Received unsupported Fast-Path Update: {0:?}")]
    UnsupportedFastPathUpdate(UpdateCode),
}
