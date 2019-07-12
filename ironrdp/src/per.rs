use std::io;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

#[cfg(test)]
mod test;

pub const SIZEOF_CHOICE: usize = 1;
pub const SIZEOF_ENUM: usize = 1;
pub const SIZEOF_U16: usize = 2;

const OBJECT_ID_LEN: usize = 6;

pub fn read_length(mut stream: impl io::Read) -> io::Result<(u16, usize)> {
    let a = stream.read_u8()?;

    if a & 0x80 != 0 {
        let b = stream.read_u8()?;
        let length = ((u16::from(a) & !0x80) << 8) + u16::from(b);

        Ok((length, 2))
    } else {
        Ok((u16::from(a), 1))
    }
}

pub fn write_length(mut stream: impl io::Write, length: u16) -> io::Result<usize> {
    if length > 0x7f {
        stream.write_u16::<BigEndian>(length | 0x8000)?;

        Ok(2)
    } else {
        stream.write_u8(length as u8)?;

        Ok(1)
    }
}

pub fn sizeof_length(length: u16) -> usize {
    if length > 0x7f {
        2
    } else {
        1
    }
}

pub fn sizeof_u32(value: u32) -> usize {
    if value <= 0xff {
        2
    } else if value <= 0xffff {
        3
    } else {
        5
    }
}

pub fn read_choice(mut stream: impl io::Read) -> io::Result<u8> {
    stream.read_u8()
}

pub fn write_choice(mut stream: impl io::Write, choice: u8) -> io::Result<usize> {
    stream.write_u8(choice)?;

    Ok(1)
}

pub fn read_selection(mut stream: impl io::Read) -> io::Result<u8> {
    stream.read_u8()
}

pub fn write_selection(mut stream: impl io::Write, selection: u8) -> io::Result<usize> {
    stream.write_u8(selection)?;

    Ok(1)
}

pub fn read_number_of_sets(mut stream: impl io::Read) -> io::Result<u8> {
    stream.read_u8()
}

pub fn write_number_of_sets(mut stream: impl io::Write, number_of_sets: u8) -> io::Result<usize> {
    stream.write_u8(number_of_sets)?;

    Ok(1)
}

pub fn read_padding(mut stream: impl io::Read, padding_length: usize) -> io::Result<()> {
    let mut buf = vec![0; padding_length];
    stream.read_exact(buf.as_mut())?;

    Ok(())
}

pub fn write_padding(mut stream: impl io::Write, padding_length: usize) -> io::Result<()> {
    let buf = vec![0; padding_length];
    stream.write_all(buf.as_ref())?;

    Ok(())
}

pub fn read_u32(mut stream: impl io::Read) -> io::Result<u32> {
    let (length, _) = read_length(&mut stream)?;

    match length {
        0 => Ok(0),
        1 => Ok(u32::from(stream.read_u8()?)),
        2 => Ok(u32::from(stream.read_u16::<BigEndian>()?)),
        4 => stream.read_u32::<BigEndian>(),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Invalid PER length: {}", length),
        )),
    }
}

pub fn write_u32(mut stream: impl io::Write, value: u32) -> io::Result<usize> {
    if value <= 0xff {
        let size = write_length(&mut stream, 1)?;
        stream.write_u8(value as u8)?;

        Ok(size + 1)
    } else if value <= 0xffff {
        let size = write_length(&mut stream, 2)?;
        stream.write_u16::<BigEndian>(value as u16)?;

        Ok(size + 2)
    } else {
        let size = write_length(&mut stream, 4)?;
        stream.write_u32::<BigEndian>(value)?;

        Ok(size + 4)
    }
}

pub fn read_u16(mut stream: impl io::Read, min: u16) -> io::Result<u16> {
    min.checked_add(stream.read_u16::<BigEndian>()?)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PER u16"))
}

pub fn write_u16(mut stream: impl io::Write, value: u16, min: u16) -> io::Result<usize> {
    if value < min {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Min is greater then number",
        ))
    } else {
        stream.write_u16::<BigEndian>(value - min)?;

        Ok(2)
    }
}

pub fn read_enum(mut stream: impl io::Read, count: u8) -> io::Result<u8> {
    let enumerated = stream.read_u8()?;

    if u16::from(enumerated) + 1 > u16::from(count) {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Enumerated value ({}) does not fall within expected range",
                enumerated
            ),
        ))
    } else {
        Ok(enumerated)
    }
}

pub fn write_enum(mut stream: impl io::Write, enumerated: u8) -> io::Result<usize> {
    stream.write_u8(enumerated)?;

    Ok(1)
}

pub fn read_object_id(mut stream: impl io::Read) -> io::Result<[u8; OBJECT_ID_LEN]> {
    let (length, _) = read_length(&mut stream)?;
    if length != 5 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid PER object id length",
        ));
    }

    let first_two_tuples = stream.read_u8()?;

    let mut read_object_ids = [0u8; OBJECT_ID_LEN];
    read_object_ids[0] = first_two_tuples / 40;
    read_object_ids[1] = first_two_tuples % 40;
    for read_object_id in read_object_ids.iter_mut().skip(2) {
        *read_object_id = stream.read_u8()?;
    }

    Ok(read_object_ids)
}

pub fn write_object_id(
    mut stream: impl io::Write,
    object_ids: [u8; OBJECT_ID_LEN],
) -> io::Result<usize> {
    let size = write_length(&mut stream, OBJECT_ID_LEN as u16 - 1)?;

    let first_two_tuples = object_ids[0] * 40 + object_ids[1];
    stream.write_u8(first_two_tuples)?;

    for object_id in object_ids.iter().skip(2) {
        stream.write_u8(*object_id)?;
    }

    Ok(size + OBJECT_ID_LEN - 1)
}

pub fn read_octet_string(mut stream: impl io::Read, min: usize) -> io::Result<Vec<u8>> {
    let (read_length, _) = read_length(&mut stream)?;

    let mut read_octet_string = vec![0; min + read_length as usize];
    stream.read_exact(read_octet_string.as_mut())?;

    Ok(read_octet_string)
}

pub fn write_octet_string(
    mut stream: impl io::Write,
    octet_string: &[u8],
    min: usize,
) -> io::Result<usize> {
    let length = if octet_string.len() >= min {
        octet_string.len() - min
    } else {
        min
    };

    let size = write_length(&mut stream, length as u16)?;
    stream.write_all(octet_string)?;

    Ok(size + octet_string.len())
}

pub fn read_numeric_string(mut stream: impl io::Read, min: u16) -> io::Result<()> {
    let (read_length, _) = read_length(&mut stream)?;

    let length = (read_length + min + 1) / 2;

    let mut read_numeric_string = vec![0; length as usize];
    stream.read_exact(read_numeric_string.as_mut())?;

    Ok(())
}

pub fn write_numeric_string(
    mut stream: impl io::Write,
    num_str: &[u8],
    min: usize,
) -> io::Result<usize> {
    let length = if num_str.len() >= min {
        num_str.len() - min
    } else {
        min
    };

    let mut size = write_length(&mut stream, length as u16)?;

    let magic_transform = |elem| (elem - 0x30) % 10;

    for pair in num_str.chunks(2) {
        let first = magic_transform(pair[0]);
        let second = magic_transform(if pair.len() == 1 { 0x30 } else { pair[1] });

        let num = (first << 4) | second;

        stream.write_u8(num)?;
        size += 1;
    }

    Ok(size)
}
