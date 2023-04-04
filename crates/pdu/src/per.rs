#![allow(dead_code)]

use crate::cursor::{ReadCursor, WriteCursor};

pub(crate) const CHOICE_SIZE: usize = 1;
pub(crate) const ENUM_SIZE: usize = 1;
pub(crate) const U16_SIZE: usize = 2;

const OBJECT_ID_SIZE: usize = 6;

pub(crate) fn read_length(src: &mut ReadCursor<'_>) -> (u16, usize) {
    let a = src.read_u8();

    if a & 0x80 != 0 {
        let b = src.read_u8();
        let length = ((u16::from(a) & !0x80) << 8) + u16::from(b);

        (length, 2)
    } else {
        (u16::from(a), 1)
    }
}

pub(crate) fn write_length(dst: &mut WriteCursor<'_>, length: u16) {
    if length > 0x7f {
        dst.write_u16_be(length | 0x8000);
    } else {
        dst.write_u8(u8::try_from(length).unwrap());
    }
}

pub(crate) fn sizeof_length(length: u16) -> usize {
    if length > 0x7f {
        2
    } else {
        1
    }
}

pub(crate) fn sizeof_u32(value: u32) -> usize {
    if value <= 0xff {
        2
    } else if value <= 0xffff {
        3
    } else {
        5
    }
}

pub(crate) fn read_choice(src: &mut ReadCursor<'_>) -> u8 {
    src.read_u8()
}

pub(crate) fn write_choice(dst: &mut WriteCursor<'_>, choice: u8) {
    dst.write_u8(choice);
}

pub(crate) fn read_selection(src: &mut ReadCursor<'_>) -> u8 {
    src.read_u8()
}

pub(crate) fn write_selection(dst: &mut WriteCursor<'_>, selection: u8) {
    dst.write_u8(selection);
}

pub(crate) fn read_number_of_sets(src: &mut ReadCursor<'_>) -> u8 {
    src.read_u8()
}

pub(crate) fn write_number_of_sets(dst: &mut WriteCursor<'_>, number_of_sets: u8) {
    dst.write_u8(number_of_sets);
}

pub(crate) fn read_padding(src: &mut ReadCursor<'_>, padding_length: usize) {
    src.advance(padding_length);
}

pub(crate) fn write_padding(dst: &mut WriteCursor<'_>, padding_length: usize) {
    for _ in 0..padding_length {
        dst.write_u8(0);
    }
}

pub(crate) fn read_u32(src: &mut ReadCursor<'_>) -> crate::Result<u32> {
    let (length, _) = read_length(src);

    match length {
        0 => Ok(0),
        1 => Ok(u32::from(src.read_u8())),
        2 => Ok(u32::from(src.read_u16_be())),
        4 => Ok(src.read_u32_be()),
        _ => Err(crate::Error::Other {
            context: "PER",
            reason: "invalid length for u32",
        }),
    }
}

pub(crate) fn write_u32(dst: &mut WriteCursor<'_>, value: u32) {
    if value <= 0xff {
        write_length(dst, 1);
        dst.write_u8(u8::try_from(value).unwrap());
    } else if value <= 0xffff {
        write_length(dst, 2);
        dst.write_u16_be(u16::try_from(value).unwrap());
    } else {
        write_length(dst, 4);
        dst.write_u32_be(value);
    }
}

pub(crate) fn read_u16(src: &mut ReadCursor<'_>, min: u16) -> crate::Result<u16> {
    min.checked_add(src.read_u16_be()).ok_or(crate::Error::Other {
        context: "PER",
        reason: "invalid u16",
    })
}

pub(crate) fn write_u16(dst: &mut WriteCursor<'_>, value: u16, min: u16) -> crate::Result<()> {
    if value < min {
        Err(crate::Error::Other {
            context: "PER",
            reason: "u16 value greater than specified minimum",
        })
    } else {
        dst.write_u16_be(value - min);
        Ok(())
    }
}

pub(crate) fn read_enum(src: &mut ReadCursor<'_>, count: u8) -> crate::Result<u8> {
    let enumerated = src.read_u8();

    if u16::from(enumerated) + 1 > u16::from(count) {
        Err(crate::Error::Other {
            context: "PER",
            reason: "enumerated value does not fall within expected range",
        })
    } else {
        Ok(enumerated)
    }
}

pub(crate) fn write_enum(dst: &mut WriteCursor<'_>, enumerated: u8) {
    dst.write_u8(enumerated);
}

pub(crate) fn read_object_id(src: &mut ReadCursor<'_>) -> crate::Result<[u8; OBJECT_ID_SIZE]> {
    let (length, _) = read_length(src);

    if length != 5 {
        return Err(crate::Error::Other {
            context: "PER",
            reason: "invalid object id length",
        });
    }

    let first_two_tuples = src.read_u8();

    let mut read_object_ids = [0u8; OBJECT_ID_SIZE];
    read_object_ids[0] = first_two_tuples / 40;
    read_object_ids[1] = first_two_tuples % 40;
    for read_object_id in read_object_ids.iter_mut().skip(2) {
        *read_object_id = src.read_u8();
    }

    Ok(read_object_ids)
}

pub(crate) fn write_object_id(dst: &mut WriteCursor<'_>, object_ids: [u8; OBJECT_ID_SIZE]) {
    write_length(dst, OBJECT_ID_SIZE as u16 - 1);

    let first_two_tuples = object_ids[0] * 40 + object_ids[1];
    dst.write_u8(first_two_tuples);

    for object_id in object_ids.iter().skip(2) {
        dst.write_u8(*object_id);
    }
}

pub(crate) fn read_octet_string(src: &mut ReadCursor<'_>, min: usize) -> Vec<u8> {
    let (length, _) = read_length(src);
    src.read_slice(min + usize::from(length)).to_owned()
}

pub(crate) fn write_octet_string(dst: &mut WriteCursor<'_>, octet_string: &[u8], min: usize) {
    let length = if octet_string.len() >= min {
        octet_string.len() - min
    } else {
        min
    };

    write_length(dst, length as u16);
    dst.write_slice(octet_string);
}

pub(crate) fn read_numeric_string(src: &mut ReadCursor<'_>, min: u16) {
    let (length, _) = read_length(src);
    let length = (length + min + 1) / 2;
    src.advance(usize::from(length))
}

pub(crate) fn write_numeric_string(dst: &mut WriteCursor<'_>, num_str: &[u8], min: usize) {
    let length = if num_str.len() >= min { num_str.len() - min } else { min };

    write_length(dst, u16::try_from(length).unwrap());

    let magic_transform = |elem| (elem - 0x30) % 10;

    for pair in num_str.chunks(2) {
        let first = magic_transform(pair[0]);
        let second = magic_transform(if pair.len() == 1 { 0x30 } else { pair[1] });

        let num = (first << 4) | second;

        dst.write_u8(num);
    }
}

pub(crate) mod legacy {
    use std::io;

    use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

    use super::OBJECT_ID_SIZE;

    pub(crate) fn read_length(mut stream: impl io::Read) -> io::Result<(u16, usize)> {
        let a = stream.read_u8()?;

        if a & 0x80 != 0 {
            let b = stream.read_u8()?;
            let length = ((u16::from(a) & !0x80) << 8) + u16::from(b);

            Ok((length, 2))
        } else {
            Ok((u16::from(a), 1))
        }
    }

    pub(crate) fn write_long_length(mut stream: impl io::Write, length: u16) -> io::Result<usize> {
        stream.write_u16::<BigEndian>(length | 0x8000)?;
        Ok(2)
    }

    pub(crate) fn write_short_length(mut stream: impl io::Write, length: u16) -> io::Result<usize> {
        stream.write_u8(length as u8)?;
        Ok(1)
    }

    pub(crate) fn write_length(stream: impl io::Write, length: u16) -> io::Result<usize> {
        if length > 0x7f {
            write_long_length(stream, length)
        } else {
            write_short_length(stream, length)
        }
    }

    pub(crate) fn read_choice(mut stream: impl io::Read) -> io::Result<u8> {
        stream.read_u8()
    }

    pub(crate) fn write_choice(mut stream: impl io::Write, choice: u8) -> io::Result<usize> {
        stream.write_u8(choice)?;

        Ok(1)
    }

    pub(crate) fn read_selection(mut stream: impl io::Read) -> io::Result<u8> {
        stream.read_u8()
    }

    pub(crate) fn write_selection(mut stream: impl io::Write, selection: u8) -> io::Result<usize> {
        stream.write_u8(selection)?;

        Ok(1)
    }

    pub(crate) fn read_number_of_sets(mut stream: impl io::Read) -> io::Result<u8> {
        stream.read_u8()
    }

    pub(crate) fn write_number_of_sets(mut stream: impl io::Write, number_of_sets: u8) -> io::Result<usize> {
        stream.write_u8(number_of_sets)?;

        Ok(1)
    }

    pub(crate) fn read_padding(mut stream: impl io::Read, padding_length: usize) -> io::Result<()> {
        let mut buf = vec![0; padding_length];
        stream.read_exact(buf.as_mut())?;

        Ok(())
    }

    pub(crate) fn write_padding(mut stream: impl io::Write, padding_length: usize) -> io::Result<()> {
        let buf = vec![0; padding_length];
        stream.write_all(buf.as_ref())?;

        Ok(())
    }

    pub(crate) fn read_u32(mut stream: impl io::Read) -> io::Result<u32> {
        let (length, _) = read_length(&mut stream)?;

        match length {
            0 => Ok(0),
            1 => Ok(u32::from(stream.read_u8()?)),
            2 => Ok(u32::from(stream.read_u16::<BigEndian>()?)),
            4 => stream.read_u32::<BigEndian>(),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid PER length: {length}"),
            )),
        }
    }

    pub(crate) fn write_u32(mut stream: impl io::Write, value: u32) -> io::Result<usize> {
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

    pub(crate) fn read_u16(mut stream: impl io::Read, min: u16) -> io::Result<u16> {
        min.checked_add(stream.read_u16::<BigEndian>()?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PER u16"))
    }

    pub(crate) fn write_u16(mut stream: impl io::Write, value: u16, min: u16) -> io::Result<usize> {
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

    pub(crate) fn read_enum(mut stream: impl io::Read, count: u8) -> io::Result<u8> {
        let enumerated = stream.read_u8()?;

        if u16::from(enumerated) + 1 > u16::from(count) {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Enumerated value ({enumerated}) does not fall within expected range"),
            ))
        } else {
            Ok(enumerated)
        }
    }

    pub(crate) fn write_enum(mut stream: impl io::Write, enumerated: u8) -> io::Result<usize> {
        stream.write_u8(enumerated)?;

        Ok(1)
    }

    pub(crate) fn read_object_id(mut stream: impl io::Read) -> io::Result<[u8; OBJECT_ID_SIZE]> {
        let (length, _) = read_length(&mut stream)?;
        if length != 5 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid PER object id length",
            ));
        }

        let first_two_tuples = stream.read_u8()?;

        let mut read_object_ids = [0u8; OBJECT_ID_SIZE];
        read_object_ids[0] = first_two_tuples / 40;
        read_object_ids[1] = first_two_tuples % 40;
        for read_object_id in read_object_ids.iter_mut().skip(2) {
            *read_object_id = stream.read_u8()?;
        }

        Ok(read_object_ids)
    }

    pub(crate) fn write_object_id(mut stream: impl io::Write, object_ids: [u8; OBJECT_ID_SIZE]) -> io::Result<usize> {
        let size = write_length(&mut stream, OBJECT_ID_SIZE as u16 - 1)?;

        let first_two_tuples = object_ids[0] * 40 + object_ids[1];
        stream.write_u8(first_two_tuples)?;

        for object_id in object_ids.iter().skip(2) {
            stream.write_u8(*object_id)?;
        }

        Ok(size + OBJECT_ID_SIZE - 1)
    }

    pub(crate) fn read_octet_string(mut stream: impl io::Read, min: usize) -> io::Result<Vec<u8>> {
        let (read_length, _) = read_length(&mut stream)?;

        let mut read_octet_string = vec![0; min + read_length as usize];
        stream.read_exact(read_octet_string.as_mut())?;

        Ok(read_octet_string)
    }

    pub(crate) fn write_octet_string(mut stream: impl io::Write, octet_string: &[u8], min: usize) -> io::Result<usize> {
        let length = if octet_string.len() >= min {
            octet_string.len() - min
        } else {
            min
        };

        let size = write_length(&mut stream, length as u16)?;
        stream.write_all(octet_string)?;

        Ok(size + octet_string.len())
    }

    pub(crate) fn read_numeric_string(mut stream: impl io::Read, min: u16) -> io::Result<()> {
        let (read_length, _) = read_length(&mut stream)?;

        let length = (read_length + min + 1) / 2;

        let mut read_numeric_string = vec![0; length as usize];
        stream.read_exact(read_numeric_string.as_mut())?;

        Ok(())
    }

    pub(crate) fn write_numeric_string(mut stream: impl io::Write, num_str: &[u8], min: usize) -> io::Result<usize> {
        let length = if num_str.len() >= min { num_str.len() - min } else { min };

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_length_is_correct_length() {
        let mut src = ReadCursor::new(&[0x05]);

        let (length, sizeof_length) = read_length(&mut src);

        assert_eq!(5, length);
        assert_eq!(src.len(), 0);
        assert_eq!(sizeof_length, 1);
    }

    #[test]
    fn read_length_is_correct_long_length() {
        let mut src = ReadCursor::new(&[0x80, 0x8d]);

        let (length, sizeof_length) = read_length(&mut src);

        assert_eq!(141, length);
        assert_eq!(src.len(), 0);
        assert_eq!(sizeof_length, 2);
    }

    #[test]
    fn write_length_is_correct() {
        let expected_buf = [0x05];

        let mut buf = [0; 1];
        let mut dst = WriteCursor::new(&mut buf);
        write_length(&mut dst, 0x05);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn write_length_is_correct_with_long_length() {
        let expected_buf = [0x80, 0x8d];

        let mut buf = [0; 2];
        let mut dst = WriteCursor::new(&mut buf);
        write_length(&mut dst, 141);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn sizeof_length_is_correct_with_small_length() {
        assert_eq!(1, sizeof_length(10));
    }

    #[test]
    fn sizeof_length_is_correct_with_long_length() {
        assert_eq!(2, sizeof_length(10_000));
    }

    #[test]
    fn read_u32_returns_correct_with_null_number() {
        let buf = [0x00];
        let mut src = ReadCursor::new(&buf);
        assert_eq!(0, read_u32(&mut src).unwrap());
    }

    #[test]
    fn read_u32_returns_correct_with_1_byte_number() {
        let buf = [0x01, 0x7f];
        let mut src = ReadCursor::new(&buf);
        assert_eq!(127, read_u32(&mut src).unwrap());
    }

    #[test]
    fn read_u32_returns_correct_with_2_bytes_number() {
        let buf = [0x02, 0x7f, 0xff];
        let mut src = ReadCursor::new(&buf);
        assert_eq!(32767, read_u32(&mut src).unwrap());
    }

    #[test]
    fn read_u32_returns_correct_with_4_bytes_number() {
        let buf = [0x04, 0x01, 0x12, 0xA8, 0x80];
        let mut src = ReadCursor::new(&buf);
        assert_eq!(18_000_000, read_u32(&mut src).unwrap());
    }

    #[test]
    fn read_u32_fails_on_invalid_length() {
        let buf = [0x03, 0x01, 0x12, 0xA8, 0x80];
        let mut src = ReadCursor::new(&buf);
        assert!(read_u32(&mut src).is_err());
    }

    #[test]
    fn write_u32_returns_correct_null_number() {
        let expected_buf = [0x01, 0x00];

        let mut buf = [0; 2];
        let mut dst = WriteCursor::new(&mut buf);
        write_u32(&mut dst, 0);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn write_u32_returns_correct_1_byte_number() {
        let expected_buf = [0x01, 0x7f];

        let mut buf = [0; 2];
        let mut dst = WriteCursor::new(&mut buf);
        write_u32(&mut dst, 127);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn write_u32_returns_correct_2_bytes_number() {
        let expected_buf = [0x02, 0x7f, 0xff];

        let mut buf = [0; 3];
        let mut dst = WriteCursor::new(&mut buf);
        write_u32(&mut dst, 32767);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn write_u32_returns_correct_4_byte_number() {
        let expected_buf = [0x04, 0x01, 0x12, 0xA8, 0x80];

        let mut buf = [0; 5];
        let mut dst = WriteCursor::new(&mut buf);
        write_u32(&mut dst, 18_000_000);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn read_u16_returns_correct_number() {
        let buf = [0x00, 0x07];
        let mut src = ReadCursor::new(&buf);
        assert_eq!(1008, read_u16(&mut src, 1001).unwrap());
    }

    #[test]
    fn read_u16_fails_on_too_big_number_with_min_value() {
        let buf = [0xff, 0xff];
        let mut src = ReadCursor::new(&buf);

        match read_u16(&mut src, 1) {
            Err(crate::Error::Other { reason, .. }) => {
                assert_eq!(reason, "invalid u16");
            }
            _ => panic!("Unexpected result"),
        };
    }

    #[test]
    fn write_u16_returns_correct_number() {
        let expected_buf = [0x00, 0x07];

        let mut buf = [0; 2];
        let mut dst = WriteCursor::new(&mut buf);
        write_u16(&mut dst, 1008, 1001).unwrap();

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn write_u16_fails_if_min_is_greater_then_number() {
        let mut buf = [0; 2];
        let mut dst = WriteCursor::new(&mut buf);

        let e = write_u16(&mut dst, 1000, 1001).err().unwrap();

        if let crate::Error::Other { context, reason } = e {
            assert_eq!(context, "PER");
            assert_eq!(reason, "u16 value greater than specified minimum");
        } else {
            panic!("unexpected error: {e}");
        }
    }

    #[test]
    fn read_object_id_returns_ok() {
        let buf = [0x05, 0x00, 0x14, 0x7c, 0x00, 0x01];
        let mut src = ReadCursor::new(&buf);
        assert_eq!([0, 0, 20, 124, 0, 1], read_object_id(&mut src).unwrap());
    }

    #[test]
    fn write_object_id_is_correct() {
        let expected_buf = [0x05, 0x00, 0x14, 0x7c, 0x00, 0x01];

        let mut buf = [0; 6];
        let mut dst = WriteCursor::new(&mut buf);
        write_object_id(&mut dst, [0, 0, 20, 124, 0, 1]);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn read_enum_fails_on_invalid_enum_with_count() {
        let buf = [0x05];
        let mut src = ReadCursor::new(&buf);

        match read_enum(&mut src, 1) {
            Err(crate::Error::Other { reason, .. }) => {
                assert_eq!(reason, "enumerated value does not fall within expected range");
            }
            _ => panic!("Unexpected result"),
        };
    }

    #[test]
    fn read_enum_returns_correct_enum() {
        let buf = [0x05];
        let mut src = ReadCursor::new(&buf);

        assert_eq!(5, read_enum(&mut src, 10).unwrap());
    }

    #[test]
    fn read_enum_fails_on_max_number() {
        let buf = [0xff];
        let mut src = ReadCursor::new(&buf);

        match read_enum(&mut src, 0xff) {
            Err(crate::Error::Other { reason, .. }) => {
                assert_eq!(reason, "enumerated value does not fall within expected range");
            }
            _ => panic!("Unexpected result"),
        };
    }

    #[test]
    fn read_numeric_string_no_panic() {
        let buf = [0x00, 0x10];
        let mut src = ReadCursor::new(&buf);

        read_numeric_string(&mut src, 1);
    }

    #[test]
    fn write_numeric_string_is_correct() {
        let expected_buf = [0x00, 0x10];
        let octet_string = b"1";

        let mut buf = [0; 2];
        let mut dst = WriteCursor::new(&mut buf);

        write_numeric_string(&mut dst, octet_string, 1);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }

    #[test]
    fn read_octet_string_returns_ok() {
        let buf = [0x00, 0x44, 0x75, 0x63, 0x61];
        let mut src = ReadCursor::new(&buf);

        assert_eq!(b"Duca", read_octet_string(&mut src, 4).as_slice());
    }

    #[test]
    fn write_octet_string_is_correct() {
        let expected_buf = [0x00, 0x44, 0x75, 0x63, 0x61];
        let octet_string = b"Duca";

        let mut buf = [0; 5];
        let mut dst = WriteCursor::new(&mut buf);

        write_octet_string(&mut dst, octet_string, 4);

        assert_eq!(dst.len(), 0);
        assert_eq!(buf, expected_buf);
    }
}
