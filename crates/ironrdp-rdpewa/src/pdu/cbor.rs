//! Minimal CBOR subset for MS-RDPEWA request/response maps.
//!
//! Multi-byte CBOR integers use network byte order (big-endian).

use ironrdp_core::{DecodeResult, EncodeResult, ReadCursor, WriteCursor, invalid_field_err, other_err};
use std::collections::BTreeMap;

const MAX_NESTING_DEPTH: usize = 32;

/// CBOR values needed by MS-RDPEWA.
#[derive(Debug, Clone, PartialEq)]
pub enum CborValue {
    Unsigned(u64),
    Negative(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    Map(BTreeMap<CborKey, CborValue>),
    Bool(bool),
    Null,
    Simple(u8),
    Float(f64),
    /// Opaque nested CBOR kept as raw bytes.
    Raw(Vec<u8>),
}

/// Map keys used by MS-RDPEWA (text or integer).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CborKey {
    Text(String),
    Int(i64),
}

impl CborKey {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }
}

pub fn decode_value(src: &mut ReadCursor<'_>) -> DecodeResult<CborValue> {
    decode_value_with_depth(src, 0)
}

fn decode_value_with_depth(src: &mut ReadCursor<'_>, depth: usize) -> DecodeResult<CborValue> {
    if src.is_empty() {
        return Err(invalid_field_err!("cbor", "input", "unexpected end of input"));
    }
    let initial = src.read_u8();
    let major = initial >> 5;
    let additional = initial & 0x1f;
    match major {
        0 => Ok(CborValue::Unsigned(read_uint(src, additional)?)),
        1 => {
            let n = read_uint(src, additional)?;
            let n_i64 = i64::try_from(n).map_err(|_| invalid_field_err!("cbor", "negative", "out of range"))?;
            let v = (-1i64)
                .checked_sub(n_i64)
                .ok_or_else(|| invalid_field_err!("cbor", "negative", "overflow"))?;
            Ok(CborValue::Negative(v))
        }
        2 => {
            let len = usize_from_u64(read_uint(src, additional)?)?;
            ensure_remaining(src, len)?;
            Ok(CborValue::Bytes(src.read_slice(len).to_vec()))
        }
        3 => {
            let len = usize_from_u64(read_uint(src, additional)?)?;
            ensure_remaining(src, len)?;
            let bytes = src.read_slice(len);
            let text = core::str::from_utf8(bytes)
                .map_err(|_| invalid_field_err!("cbor", "text", "not utf-8"))?
                .to_owned();
            Ok(CborValue::Text(text))
        }
        4 => {
            ensure_nesting_depth(depth)?;
            let len = usize_from_u64(read_uint(src, additional)?)?;
            let mut items = Vec::with_capacity(len.min(64));
            for _ in 0..len {
                items.push(decode_value_with_depth(src, depth + 1)?);
            }
            Ok(CborValue::Array(items))
        }
        5 => {
            ensure_nesting_depth(depth)?;
            let len = usize_from_u64(read_uint(src, additional)?)?;
            let mut map = BTreeMap::new();
            for _ in 0..len {
                let key = decode_key(src, depth + 1)?;
                let value = decode_value_with_depth(src, depth + 1)?;
                map.insert(key, value);
            }
            Ok(CborValue::Map(map))
        }
        7 => match additional {
            20 => Ok(CborValue::Bool(false)),
            21 => Ok(CborValue::Bool(true)),
            22 => Ok(CborValue::Null),
            25 => {
                ensure_remaining(src, 2)?;
                let bits = src.read_u16_be();
                Ok(CborValue::Float(f64::from(half_to_f32(bits))))
            }
            26 => {
                ensure_remaining(src, 4)?;
                let bits = src.read_u32_be();
                Ok(CborValue::Float(f64::from(f32::from_bits(bits))))
            }
            27 => {
                ensure_remaining(src, 8)?;
                let bits = src.read_u64_be();
                Ok(CborValue::Float(f64::from_bits(bits)))
            }
            24 => {
                ensure_remaining(src, 1)?;
                Ok(CborValue::Simple(src.read_u8()))
            }
            n if n < 24 => Ok(CborValue::Simple(n)),
            _ => Err(invalid_field_err!("cbor", "simple", "unsupported simple/float value")),
        },
        _ => Err(invalid_field_err!("cbor", "major", "unsupported major type")),
    }
}

fn ensure_nesting_depth(depth: usize) -> DecodeResult<()> {
    if depth >= MAX_NESTING_DEPTH {
        Err(invalid_field_err!("cbor", "input", "maximum nesting depth exceeded"))
    } else {
        Ok(())
    }
}

fn decode_key(src: &mut ReadCursor<'_>, depth: usize) -> DecodeResult<CborKey> {
    match decode_value_with_depth(src, depth)? {
        CborValue::Text(s) => Ok(CborKey::Text(s)),
        CborValue::Unsigned(n) => {
            let v = i64::try_from(n).map_err(|_| invalid_field_err!("cbor", "key", "int out of range"))?;
            Ok(CborKey::Int(v))
        }
        CborValue::Negative(n) => Ok(CborKey::Int(n)),
        _ => Err(invalid_field_err!("cbor", "key", "unsupported map key type")),
    }
}

pub fn encode_value(dst: &mut WriteCursor<'_>, value: &CborValue) -> EncodeResult<()> {
    match value {
        CborValue::Unsigned(n) => write_type_uint(dst, 0, *n),
        CborValue::Negative(n) => {
            let stored = (-1i64)
                .checked_sub(*n)
                .ok_or_else(|| other_err!("cbor", "negative encode overflow"))?;
            let stored = u64::try_from(stored).map_err(|_| other_err!("cbor", "negative encode range"))?;
            write_type_uint(dst, 1, stored)
        }
        CborValue::Bytes(b) => {
            write_type_uint(dst, 2, u64_from_usize(b.len())?)?;
            dst.write_slice(b);
            Ok(())
        }
        CborValue::Text(s) => {
            write_type_uint(dst, 3, u64_from_usize(s.len())?)?;
            dst.write_slice(s.as_bytes());
            Ok(())
        }
        CborValue::Array(items) => {
            write_type_uint(dst, 4, u64_from_usize(items.len())?)?;
            for item in items {
                encode_value(dst, item)?;
            }
            Ok(())
        }
        CborValue::Map(map) => {
            write_type_uint(dst, 5, u64_from_usize(map.len())?)?;
            for (key, val) in map {
                encode_key(dst, key)?;
                encode_value(dst, val)?;
            }
            Ok(())
        }
        CborValue::Bool(false) => {
            dst.write_u8(0xf4);
            Ok(())
        }
        CborValue::Bool(true) => {
            dst.write_u8(0xf5);
            Ok(())
        }
        CborValue::Null => {
            dst.write_u8(0xf6);
            Ok(())
        }
        CborValue::Simple(n) => {
            if *n < 24 {
                dst.write_u8(0xe0 | n);
            } else {
                dst.write_u8(0xf8);
                dst.write_u8(*n);
            }
            Ok(())
        }
        CborValue::Float(f) => {
            dst.write_u8(0xfb);
            dst.write_u64_be(f.to_bits());
            Ok(())
        }
        CborValue::Raw(bytes) => {
            dst.write_slice(bytes);
            Ok(())
        }
    }
}

fn encode_key(dst: &mut WriteCursor<'_>, key: &CborKey) -> EncodeResult<()> {
    match key {
        CborKey::Text(s) => encode_value(dst, &CborValue::Text(s.clone())),
        CborKey::Int(n) if *n >= 0 => {
            let unsigned = u64::try_from(*n).map_err(|_| other_err!("cbor", "key int out of range"))?;
            encode_value(dst, &CborValue::Unsigned(unsigned))
        }
        CborKey::Int(n) => encode_value(dst, &CborValue::Negative(*n)),
    }
}

pub fn encoded_size(value: &CborValue) -> usize {
    match value {
        CborValue::Unsigned(n) => 1 + uint_extra_len(*n),
        CborValue::Negative(v) => {
            let n = negative_to_stored_uint(*v).unwrap_or(u64::MAX);
            1 + uint_extra_len(n)
        }
        CborValue::Bytes(b) => 1 + uint_extra_len(u64_from_usize_lossy(b.len())) + b.len(),
        CborValue::Text(s) => 1 + uint_extra_len(u64_from_usize_lossy(s.len())) + s.len(),
        CborValue::Array(items) => {
            1 + uint_extra_len(u64_from_usize_lossy(items.len())) + items.iter().map(encoded_size).sum::<usize>()
        }
        CborValue::Map(map) => {
            1 + uint_extra_len(u64_from_usize_lossy(map.len()))
                + map.iter().map(|(k, v)| key_size(k) + encoded_size(v)).sum::<usize>()
        }
        CborValue::Bool(_) | CborValue::Null => 1,
        CborValue::Simple(n) if *n < 24 => 1,
        CborValue::Simple(_) => 2,
        CborValue::Float(_) => 9,
        CborValue::Raw(b) => b.len(),
    }
}

fn key_size(key: &CborKey) -> usize {
    match key {
        CborKey::Text(s) => encoded_size(&CborValue::Text(s.clone())),
        CborKey::Int(n) if *n >= 0 => {
            let unsigned = u64::try_from(*n).unwrap_or(u64::MAX);
            encoded_size(&CborValue::Unsigned(unsigned))
        }
        CborKey::Int(n) => encoded_size(&CborValue::Negative(*n)),
    }
}

fn uint_extra_len(n: u64) -> usize {
    if n < 24 {
        0
    } else if u8::try_from(n).is_ok() {
        1
    } else if u16::try_from(n).is_ok() {
        2
    } else if u32::try_from(n).is_ok() {
        4
    } else {
        8
    }
}

fn write_type_uint(dst: &mut WriteCursor<'_>, major: u8, n: u64) -> EncodeResult<()> {
    let major_shift = major << 5;
    if n < 24 {
        let n_u8 = u8::try_from(n).map_err(|_| other_err!("cbor", "uint additional info out of range"))?;
        dst.write_u8(major_shift | n_u8);
    } else if let Ok(n_u8) = u8::try_from(n) {
        dst.write_u8(major_shift | 24);
        dst.write_u8(n_u8);
    } else if let Ok(n_u16) = u16::try_from(n) {
        dst.write_u8(major_shift | 25);
        dst.write_u16_be(n_u16);
    } else if let Ok(n_u32) = u32::try_from(n) {
        dst.write_u8(major_shift | 26);
        dst.write_u32_be(n_u32);
    } else {
        dst.write_u8(major_shift | 27);
        dst.write_u64_be(n);
    }
    Ok(())
}

fn u64_from_usize(n: usize) -> EncodeResult<u64> {
    u64::try_from(n).map_err(|_| other_err!("cbor", "length does not fit in u64"))
}

fn u64_from_usize_lossy(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn negative_to_stored_uint(v: i64) -> EncodeResult<u64> {
    let stored = (-1i64)
        .checked_sub(v)
        .ok_or_else(|| other_err!("cbor", "negative encode overflow"))?;
    u64::try_from(stored).map_err(|_| other_err!("cbor", "negative encode range"))
}

fn read_uint(src: &mut ReadCursor<'_>, additional: u8) -> DecodeResult<u64> {
    match additional {
        n if n < 24 => Ok(u64::from(n)),
        24 => {
            ensure_remaining(src, 1)?;
            Ok(u64::from(src.read_u8()))
        }
        25 => {
            ensure_remaining(src, 2)?;
            Ok(u64::from(src.read_u16_be()))
        }
        26 => {
            ensure_remaining(src, 4)?;
            Ok(u64::from(src.read_u32_be()))
        }
        27 => {
            ensure_remaining(src, 8)?;
            Ok(src.read_u64_be())
        }
        _ => Err(invalid_field_err!(
            "cbor",
            "uint",
            "unsupported integer additional info"
        )),
    }
}

fn usize_from_u64(n: u64) -> DecodeResult<usize> {
    usize::try_from(n).map_err(|_| invalid_field_err!("cbor", "length", "does not fit in usize"))
}

fn ensure_remaining(src: &ReadCursor<'_>, n: usize) -> DecodeResult<()> {
    if src.len() < n {
        Err(invalid_field_err!("cbor", "input", "not enough bytes"))
    } else {
        Ok(())
    }
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = (bits >> 15) & 1;
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x3ff;
    let sign_f = if sign == 0 { 1.0f32 } else { -1.0f32 };
    if exp == 0 {
        if frac == 0 {
            return 0.0 * sign_f;
        }
        return sign_f * f32::from(frac) * 2f32.powi(-24);
    }
    if exp == 31 {
        if frac == 0 {
            return sign_f * f32::INFINITY;
        }
        return f32::NAN;
    }
    sign_f * (1.0 + f32::from(frac) / 1024.0) * 2f32.powi(i32::from(exp) - 15)
}

pub fn encode_to_vec(value: &CborValue) -> EncodeResult<Vec<u8>> {
    let mut buf = vec![0u8; encoded_size(value)];
    let mut cursor = WriteCursor::new(&mut buf);
    encode_value(&mut cursor, value)?;
    Ok(buf)
}

pub fn decode_all(data: &[u8]) -> DecodeResult<CborValue> {
    let mut cursor = ReadCursor::new(data);
    let value = decode_value(&mut cursor)?;
    if !cursor.is_empty() {
        return Err(invalid_field_err!("cbor", "input", "trailing bytes after value"));
    }
    Ok(value)
}

impl CborValue {
    pub fn as_u32(&self) -> DecodeResult<u32> {
        match self {
            CborValue::Unsigned(n) => {
                u32::try_from(*n).map_err(|_| invalid_field_err!("cbor", "uint", "u32 out of range"))
            }
            _ => Err(invalid_field_err!("cbor", "uint", "expected unsigned integer")),
        }
    }

    pub fn as_i64(&self) -> DecodeResult<i64> {
        match self {
            CborValue::Unsigned(n) => {
                i64::try_from(*n).map_err(|_| invalid_field_err!("cbor", "int", "i64 out of range"))
            }
            CborValue::Negative(n) => Ok(*n),
            _ => Err(invalid_field_err!("cbor", "int", "expected integer")),
        }
    }

    pub fn as_u64(&self) -> DecodeResult<u64> {
        match self {
            CborValue::Unsigned(n) => Ok(*n),
            _ => Err(invalid_field_err!("cbor", "uint", "expected unsigned integer")),
        }
    }

    pub fn as_bool(&self) -> DecodeResult<bool> {
        match self {
            CborValue::Bool(b) => Ok(*b),
            _ => Err(invalid_field_err!("cbor", "bool", "expected boolean")),
        }
    }

    pub fn as_bytes(&self) -> DecodeResult<&[u8]> {
        match self {
            CborValue::Bytes(b) => Ok(b),
            _ => Err(invalid_field_err!("cbor", "bytes", "expected byte string")),
        }
    }

    pub fn as_text(&self) -> DecodeResult<&str> {
        match self {
            CborValue::Text(s) => Ok(s),
            _ => Err(invalid_field_err!("cbor", "text", "expected text string")),
        }
    }

    pub fn as_map(&self) -> DecodeResult<&BTreeMap<CborKey, CborValue>> {
        match self {
            CborValue::Map(m) => Ok(m),
            _ => Err(invalid_field_err!("cbor", "map", "expected map")),
        }
    }

    pub fn into_map(self) -> DecodeResult<BTreeMap<CborKey, CborValue>> {
        match self {
            CborValue::Map(m) => Ok(m),
            _ => Err(invalid_field_err!("cbor", "map", "expected map")),
        }
    }

    pub fn as_array(&self) -> DecodeResult<&[CborValue]> {
        match self {
            CborValue::Array(a) => Ok(a),
            _ => Err(invalid_field_err!("cbor", "array", "expected array")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_simple_map() {
        let mut map = BTreeMap::new();
        map.insert(CborKey::text("command"), CborValue::Unsigned(8));
        map.insert(CborKey::text("flags"), CborValue::Unsigned(0));
        let value = CborValue::Map(map);
        let encoded = encode_to_vec(&value).unwrap();
        let decoded = decode_all(&encoded).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn decode_api_version_request() {
        let mut map = BTreeMap::new();
        map.insert(CborKey::text("command"), CborValue::Unsigned(8));
        map.insert(CborKey::text("flags"), CborValue::Unsigned(0));
        map.insert(CborKey::text("timeout"), CborValue::Unsigned(0));
        map.insert(CborKey::text("transactionId"), CborValue::Bytes(vec![0; 16]));
        let encoded = encode_to_vec(&CborValue::Map(map)).unwrap();
        let decoded = decode_all(&encoded).unwrap();
        let m = decoded.as_map().unwrap();
        assert_eq!(m.get(&CborKey::text("command")).unwrap().as_u32().unwrap(), 8);
    }

    #[test]
    fn rejects_excessively_nested_values() {
        let mut encoded = vec![0x81; MAX_NESTING_DEPTH + 1];
        encoded.push(0xf6);

        let error = decode_all(&encoded).expect_err("nested CBOR must be rejected");
        assert!(error.to_string().contains("maximum nesting depth"), "{error}");
    }
}
