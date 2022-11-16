#[cfg(test)]
mod tests;

use std::io;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

#[repr(u8)]
#[allow(unused)]
pub enum Pc {
    Primitive = 0x00,
    Construct = 0x20,
}

#[repr(u8)]
#[allow(unused)]
enum Class {
    Universal = 0x00,
    Application = 0x40,
    ContextSpecific = 0x80,
    Private = 0xC0,
}

#[repr(u8)]
#[allow(unused)]
enum Tag {
    Mask = 0x1F,
    Boolean = 0x01,
    Integer = 0x02,
    BitString = 0x03,
    OctetString = 0x04,
    ObjectIdentifier = 0x06,
    Enumerated = 0x0A,
    Sequence = 0x10,
}

pub const SIZEOF_ENUMERATED: u16 = 3;
pub const SIZEOF_BOOL: u16 = 3;

const TAG_MASK: u8 = 0x1F;

pub fn sizeof_application_tag(tagnum: u8, length: u16) -> u16 {
    let tag_len = if tagnum > 0x1E { 2 } else { 1 };

    sizeof_length(length) + tag_len
}

pub fn sizeof_sequence_tag(length: u16) -> u16 {
    1 + sizeof_length(length)
}

pub fn sizeof_octet_string(length: u16) -> u16 {
    1 + sizeof_length(length) + length
}

pub fn sizeof_integer(value: u32) -> u16 {
    if value < 0x0000_0080 {
        3
    } else if value < 0x0000_8000 {
        4
    } else if value < 0x0080_0000 {
        5
    } else {
        6
    }
}

pub fn write_sequence_tag(mut stream: impl io::Write, length: u16) -> io::Result<usize> {
    write_universal_tag(&mut stream, Tag::Sequence, Pc::Construct)?;
    write_length(stream, length).map(|length| length + 1)
}

pub fn read_sequence_tag(mut stream: impl io::Read) -> io::Result<u16> {
    let identifier = stream.read_u8()?;

    if identifier != Class::Universal as u8 | Pc::Construct as u8 | (TAG_MASK & Tag::Sequence as u8) {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid sequence tag identifier",
        ))
    } else {
        read_length(stream)
    }
}

pub fn write_application_tag(mut stream: impl io::Write, tagnum: u8, length: u16) -> io::Result<usize> {
    let taglen = if tagnum > 0x1E {
        stream.write_u8(Class::Application as u8 | Pc::Construct as u8 | TAG_MASK)?;
        stream.write_u8(tagnum)?;
        2
    } else {
        stream.write_u8(Class::Application as u8 | Pc::Construct as u8 | (TAG_MASK & tagnum))?;
        1
    };

    write_length(stream, length).map(|length| length + taglen)
}

pub fn read_application_tag(mut stream: impl io::Read, tagnum: u8) -> io::Result<u16> {
    let identifier = stream.read_u8()?;

    if tagnum > 0x1E {
        if identifier != Class::Application as u8 | Pc::Construct as u8 | TAG_MASK {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid application tag identifier",
            ));
        }
        if stream.read_u8()? != tagnum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid application tag identifier",
            ));
        }
    } else if identifier != Class::Application as u8 | Pc::Construct as u8 | (TAG_MASK & tagnum) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid application tag identifier",
        ));
    }

    read_length(stream)
}

pub fn write_enumerated(mut stream: impl io::Write, enumerated: u8) -> io::Result<usize> {
    let mut size = 0;
    size += write_universal_tag(&mut stream, Tag::Enumerated, Pc::Primitive)?;
    size += write_length(&mut stream, 1)?;
    stream.write_u8(enumerated)?;
    size += 1;

    Ok(size)
}

pub fn read_enumerated(mut stream: impl io::Read, count: u8) -> io::Result<u8> {
    read_universal_tag(&mut stream, Tag::Enumerated, Pc::Primitive)?;

    let length = read_length(&mut stream)?;
    if length != 1 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid enumerated len"));
    }

    let enumerated = stream.read_u8()?;
    if enumerated == std::u8::MAX || enumerated + 1 > count {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid enumerated value"));
    }

    Ok(enumerated)
}

pub fn write_integer(mut stream: impl io::Write, value: u32) -> io::Result<usize> {
    write_universal_tag(&mut stream, Tag::Integer, Pc::Primitive)?;

    if value < 0x0000_0080 {
        write_length(&mut stream, 1)?;
        stream.write_u8(value as u8)?;

        Ok(3)
    } else if value < 0x0000_8000 {
        write_length(&mut stream, 2)?;
        stream.write_u16::<BigEndian>(value as u16)?;

        Ok(4)
    } else if value < 0x0080_0000 {
        write_length(&mut stream, 3)?;
        stream.write_u8((value >> 16) as u8)?;
        stream.write_u16::<BigEndian>((value & 0xFFFF) as u16)?;

        Ok(5)
    } else {
        write_length(&mut stream, 4)?;
        stream.write_u32::<BigEndian>(value)?;

        Ok(6)
    }
}

pub fn read_integer(mut stream: impl io::Read) -> io::Result<u64> {
    read_universal_tag(&mut stream, Tag::Integer, Pc::Primitive)?;
    let length = read_length(&mut stream)?;

    if length == 1 {
        stream.read_u8().map(u64::from)
    } else if length == 2 {
        stream.read_u16::<BigEndian>().map(u64::from)
    } else if length == 3 {
        let a = stream.read_u8()?;
        let b = stream.read_u16::<BigEndian>()?;

        Ok(u64::from(b) + (u64::from(a) << 16))
    } else if length == 4 {
        stream.read_u32::<BigEndian>().map(u64::from)
    } else if length == 8 {
        stream.read_u64::<BigEndian>()
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "invalid integer len"))
    }
}

pub fn write_bool(mut stream: impl io::Write, value: bool) -> io::Result<usize> {
    let mut size = 0;
    size += write_universal_tag(&mut stream, Tag::Boolean, Pc::Primitive)?;
    size += write_length(&mut stream, 1)?;
    stream.write_u8(if value { 0xFF } else { 0x00 })?;
    size += 1;

    Ok(size)
}

pub fn read_bool(mut stream: impl io::Read) -> io::Result<bool> {
    read_universal_tag(&mut stream, Tag::Boolean, Pc::Primitive)?;
    let length = read_length(&mut stream)?;

    if length != 1 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid integer len"));
    }

    Ok(stream.read_u8()? != 0)
}

pub fn write_octet_string(mut stream: impl io::Write, value: &[u8]) -> io::Result<usize> {
    let tag_size = write_octet_string_tag(&mut stream, value.len() as u16)?;
    stream.write_all(value)?;
    Ok(tag_size + value.len())
}

pub fn write_octet_string_tag(mut stream: impl io::Write, length: u16) -> io::Result<usize> {
    write_universal_tag(&mut stream, Tag::OctetString, Pc::Primitive)?;
    write_length(&mut stream, length).map(|length| length + 1)
}

pub fn read_octet_string(mut stream: impl io::Read) -> io::Result<Vec<u8>> {
    let length = read_octet_string_tag(&mut stream)?;

    let mut buffer = vec![0; length as usize];
    stream.read_exact(&mut buffer)?;

    Ok(buffer)
}

pub fn read_octet_string_tag(mut stream: impl io::Read) -> io::Result<u16> {
    read_universal_tag(&mut stream, Tag::OctetString, Pc::Primitive)?;
    read_length(stream)
}

fn write_universal_tag(mut stream: impl io::Write, tag: Tag, pc: Pc) -> io::Result<usize> {
    let identifier = Class::Universal as u8 | pc as u8 | (TAG_MASK & tag as u8);
    stream.write_u8(identifier)?;

    Ok(1)
}

fn read_universal_tag(mut stream: impl io::Read, tag: Tag, pc: Pc) -> io::Result<()> {
    let identifier = stream.read_u8()?;

    if identifier != Class::Universal as u8 | pc as u8 | (TAG_MASK & tag as u8) {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid universal tag identifier",
        ))
    } else {
        Ok(())
    }
}

fn write_length(mut stream: impl io::Write, length: u16) -> io::Result<usize> {
    if length > 0xFF {
        stream.write_u8(0x80 ^ 0x2)?;
        stream.write_u16::<BigEndian>(length)?;

        Ok(3)
    } else if length > 0x7F {
        stream.write_u8(0x80 ^ 0x1)?;
        stream.write_u8(length as u8)?;

        Ok(2)
    } else {
        stream.write_u8(length as u8)?;

        Ok(1)
    }
}

fn read_length(mut stream: impl io::Read) -> io::Result<u16> {
    let byte = stream.read_u8()?;

    if byte & 0x80 != 0 {
        let len = byte & !0x80;

        if len == 1 {
            stream.read_u8().map(u16::from)
        } else if len == 2 {
            stream.read_u16::<BigEndian>()
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid length of the length",
            ))
        }
    } else {
        Ok(u16::from(byte))
    }
}

fn sizeof_length(length: u16) -> u16 {
    if length > 0xff {
        3
    } else if length > 0x7f {
        2
    } else {
        1
    }
}
