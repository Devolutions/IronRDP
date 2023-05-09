use std::io;

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::ToPrimitive as _;

pub(crate) fn to_utf16_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(|i| i.to_le_bytes().to_vec())
        .collect::<Vec<u8>>()
}

pub(crate) fn from_utf16_bytes(mut value: &[u8]) -> String {
    let mut value_u16 = vec![0x00; value.len() / 2];
    value
        .read_u16_into::<LittleEndian>(value_u16.as_mut())
        .expect("read_u16_into cannot fail at this point");

    String::from_utf16_lossy(value_u16.as_ref())
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub(crate) enum CharacterSet {
    Ansi = 1,
    Unicode = 2,
}

pub(crate) fn read_string(
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
        CharacterSet::Unicode => from_utf16_bytes(buffer.as_slice()),
        CharacterSet::Ansi => String::from_utf8(buffer)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("the string is not utf8: {e}")))?,
    };

    Ok(result.trim_end_matches('\0').into())
}

pub(crate) fn write_string_with_null_terminator(
    mut stream: impl io::Write,
    value: &str,
    character_set: CharacterSet,
) -> io::Result<()> {
    match character_set {
        CharacterSet::Unicode => {
            stream.write_all(to_utf16_bytes(value).as_ref())?;
            stream.write_u16::<LittleEndian>(0)
        }
        CharacterSet::Ansi => {
            stream.write_all(value.as_bytes())?;
            stream.write_u8(0)
        }
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
