pub mod rc4;
pub mod rsa;

use std::cmp::{max, min};
use std::{io, ops};

use bitvec::prelude::{BitSlice, Msb0};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::ToPrimitive;

use crate::PduParsing;

#[macro_export]
macro_rules! try_read_optional {
    ($e:expr, $ret:expr) => {
        match $e {
            Ok(v) => v,
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok($ret);
            }
            Err(e) => return Err(From::from(e)),
        }
    };
}

#[macro_export]
macro_rules! try_write_optional {
    ($val:expr, $f:expr) => {
        if let Some(ref val) = $val {
            $f(val)?
        } else {
            return Ok(());
        }
    };
}

#[macro_export]
macro_rules! impl_from_error {
    ($from_e:ty, $to_e:ty, $to_e_variant:expr) => {
        impl From<$from_e> for $to_e {
            fn from(e: $from_e) -> Self {
                $to_e_variant(e)
            }
        }
    };
}

pub fn string_to_utf16(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(|i| i.to_le_bytes().to_vec())
        .collect::<Vec<u8>>()
}

pub fn bytes_to_utf16_string(mut value: &[u8]) -> String {
    let mut value_u16 = vec![0x00; value.len() / 2];
    value
        .read_u16_into::<LittleEndian>(value_u16.as_mut())
        .expect("read_u16_into cannot fail at this point");

    String::from_utf16_lossy(value_u16.as_ref())
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum CharacterSet {
    Ansi = 1,
    Unicode = 2,
}

pub fn read_string(
    mut stream: impl io::Read,
    size: usize,
    character_set: CharacterSet,
    read_null_terminator: bool,
) -> io::Result<String> {
    let size = size
        + if read_null_terminator {
            character_set.to_usize().unwrap()
        } else {
            0
        };
    let mut buffer = vec![0; size];
    stream.read_exact(&mut buffer)?;

    let result = match character_set {
        CharacterSet::Unicode => bytes_to_utf16_string(buffer.as_slice()),
        CharacterSet::Ansi => String::from_utf8(buffer)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("the string is not utf8: {}", e)))?,
    };

    Ok(result.trim_end_matches('\0').into())
}

pub fn write_string_with_null_terminator(
    mut stream: impl io::Write,
    value: &str,
    character_set: CharacterSet,
) -> io::Result<()> {
    match character_set {
        CharacterSet::Unicode => {
            stream.write_all(string_to_utf16(value).as_ref())?;
            stream.write_u16::<LittleEndian>(0)
        }
        CharacterSet::Ansi => {
            stream.write_all(value.as_bytes())?;
            stream.write_u8(0)
        }
    }
}

pub struct Bits<'a> {
    bits_slice: &'a BitSlice<u8, Msb0>,
    remaining_bits_of_last_byte: usize,
}

impl<'a> Bits<'a> {
    pub fn new(bits_slice: &'a BitSlice<u8, Msb0>) -> Self {
        Self {
            bits_slice,
            remaining_bits_of_last_byte: 0,
        }
    }

    pub fn split_to(&mut self, at: usize) -> &'a BitSlice<u8, Msb0> {
        let (value, new_bits) = self.bits_slice.split_at(at);
        self.bits_slice = new_bits;
        self.remaining_bits_of_last_byte = (self.remaining_bits_of_last_byte + at) % 8;
        value
    }

    pub fn remaining_bits_of_last_byte(&self) -> usize {
        self.remaining_bits_of_last_byte
    }
}

impl<'a> ops::Deref for Bits<'a> {
    type Target = BitSlice<u8, Msb0>;

    fn deref(&self) -> &Self::Target {
        self.bits_slice
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rectangle {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl Rectangle {
    pub fn empty() -> Self {
        Self {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }
    }

    pub fn width(&self) -> u16 {
        self.right - self.left
    }

    pub fn height(&self) -> u16 {
        self.bottom - self.top
    }

    pub fn union_all(rectangles: &[Self]) -> Self {
        Self {
            left: rectangles.iter().map(|r| r.left).min().unwrap_or(0),
            top: rectangles.iter().map(|r| r.top).min().unwrap_or(0),
            right: rectangles.iter().map(|r| r.right).max().unwrap_or(0),
            bottom: rectangles.iter().map(|r| r.bottom).max().unwrap_or(0),
        }
    }

    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let result = Self {
            left: max(self.left, other.left),
            top: max(self.top, other.top),
            right: min(self.right, other.right),
            bottom: min(self.bottom, other.bottom),
        };

        if result.left < result.right && result.top < result.bottom {
            Some(result)
        } else {
            None
        }
    }

    pub fn union(&self, other: &Self) -> Self {
        Self {
            left: min(self.left, other.left),
            top: min(self.top, other.top),
            right: max(self.right, other.right),
            bottom: max(self.bottom, other.bottom),
        }
    }
}

impl PduParsing for Rectangle {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let left = stream.read_u16::<LittleEndian>()?;
        let top = stream.read_u16::<LittleEndian>()?;
        let right = stream.read_u16::<LittleEndian>()?;
        let bottom = stream.read_u16::<LittleEndian>()?;

        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.left)?;
        stream.write_u16::<LittleEndian>(self.top)?;
        stream.write_u16::<LittleEndian>(self.right)?;
        stream.write_u16::<LittleEndian>(self.bottom)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        8
    }
}

pub trait SplitTo {
    fn split_to(&mut self, n: usize) -> Self;
}

impl<T> SplitTo for &[T] {
    fn split_to(&mut self, n: usize) -> Self {
        assert!(n <= self.len());

        let (a, b) = self.split_at(n);
        *self = b;

        a
    }
}

impl<T> SplitTo for &mut [T] {
    fn split_to(&mut self, n: usize) -> Self {
        assert!(n <= self.len());

        let (a, b) = std::mem::take(self).split_at_mut(n);
        *self = b;

        a
    }
}
