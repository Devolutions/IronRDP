use core::fmt;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
    unexpected_message_type_err, unsupported_version_err,
};

const PACKAGE_HEADER: [u8; 16] = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Decrypted version-zero [TSRemoteGuardInnerPacket].
///
/// Decoding borrows the input without allocating and validates the package buffer header.
/// `package_call` excludes that header and remains opaque; it is not a validated NDR call.
/// The default version is omitted in DER; currently unused extensions are unsupported.
/// This codec neither authenticates the input nor authorizes its security package.
///
/// [TSRemoteGuardInnerPacket]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpear/
#[derive(PartialEq, Eq)]
pub struct RemoteGuardInnerPacket<'a> {
    pub package_name: &'a [u8],
    pub package_call: &'a [u8],
}

impl RemoteGuardInnerPacket<'_> {
    const NAME: &'static str = "RemoteGuardInnerPacket";
}

impl<'a> RemoteGuardInnerPacket<'a> {
    /// Checks the complete plaintext limit before parsing.
    ///
    /// The caller must also bound decryption allocations before producing this plaintext.
    pub fn decode_with_limit(src: &mut ReadCursor<'a>, max_plaintext_size: usize) -> DecodeResult<Self> {
        if max_plaintext_size < src.len() {
            return Err(invalid_field_err!("plaintext", "inner packet exceeds configured limit", in: src));
        }
        Self::decode(src)
    }
}

impl fmt::Debug for RemoteGuardInnerPacket<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(Self::NAME)
            .field("package_name_length", &self.package_name.len())
            .field("package_call_length", &self.package_call.len())
            .finish()
    }
}

impl<'de> Decode<'de> for RemoteGuardInnerPacket<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let length = read_header(src, 0x30)?;
        if length != src.len() {
            return Err(invalid_field_err!("sequence", "expected exactly one complete inner packet", in: src));
        }

        if src.remaining().first() == Some(&0xa0) {
            let version_length = read_header(src, 0xa0)?;
            let version_end = src.pos() + version_length;
            if read_header(src, 0x0a)? != 1 || src.pos() + 1 != version_end {
                return Err(invalid_field_err!("version", "invalid version enumeration", in: src));
            }
            let version = src.read_u8();
            if version != 0 {
                return Err(unsupported_version_err!(version, in: src));
            }
            return Err(invalid_field_err!("version", "DER default version must be omitted", in: src));
        }

        let package_name = read_octets(src, 0xa1)?;
        let buffer = read_octets(src, 0xa2)?;
        if let Some(tag) = src.remaining().first() {
            return Err(unexpected_message_type_err!("extension tag", *tag, in: src));
        }
        if !buffer.starts_with(&PACKAGE_HEADER) {
            return Err(invalid_field_err!("buffer", "invalid security package buffer header", in: src));
        }
        Ok(Self {
            package_name,
            package_call: &buffer[PACKAGE_HEADER.len()..],
        })
    }
}

impl Encode for RemoteGuardInnerPacket<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = self.size();
        if size == usize::MAX {
            return Err(invalid_field_err!("length", "inner packet size overflow", in: dst));
        }
        ensure_size!(in: dst, size: size);
        let buffer_length = PACKAGE_HEADER.len() + self.package_call.len();
        let package_length = tlv_size(self.package_name.len());
        let buffer_field_length = tlv_size(buffer_length);
        write_header(dst, 0x30, tlv_size(package_length) + tlv_size(buffer_field_length));
        write_header(dst, 0xa1, package_length);
        write_header(dst, 0x04, self.package_name.len());
        dst.write_slice(self.package_name);
        write_header(dst, 0xa2, buffer_field_length);
        write_header(dst, 0x04, buffer_length);
        dst.write_slice(&PACKAGE_HEADER);
        dst.write_slice(self.package_call);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let package_length = tlv_size(tlv_size(self.package_name.len()));
        let buffer_length = tlv_size(tlv_size(PACKAGE_HEADER.len().saturating_add(self.package_call.len())));
        tlv_size(package_length.saturating_add(buffer_length))
    }
}

fn read_header(src: &mut ReadCursor<'_>, tag: u8) -> DecodeResult<usize> {
    ensure_size!(in: src, size: 2);
    if src.read_u8() != tag {
        return Err(invalid_field_err!("tag", "unexpected DER tag", in: src));
    }
    let first = src.read_u8();
    let length = if first < 0x80 {
        usize::from(first)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || size_of::<usize>() < count {
            return Err(invalid_field_err!("length", "invalid DER length encoding", in: src));
        }
        ensure_size!(in: src, size: count);
        let bytes = src.read_slice(count);
        if bytes[0] == 0 {
            return Err(invalid_field_err!("length", "nonminimal DER length", in: src));
        }
        let mut length = 0usize;
        for byte in bytes {
            length = (length << 8) | usize::from(*byte);
        }
        if length < 0x80 {
            return Err(invalid_field_err!("length", "nonminimal DER length", in: src));
        }
        length
    };
    ensure_size!(in: src, size: length);
    Ok(length)
}

fn read_octets<'a>(src: &mut ReadCursor<'a>, tag: u8) -> DecodeResult<&'a [u8]> {
    let field_length = read_header(src, tag)?;
    let end = src.pos() + field_length;
    let length = read_header(src, 0x04)?;
    if src.pos() + length != end {
        return Err(invalid_field_err!("field", "invalid explicit OCTET STRING length", in: src));
    }
    Ok(src.read_slice(length))
}

fn tlv_size(length: usize) -> usize {
    let length_size = if length < 0x80 {
        1
    } else {
        1 + length.to_be_bytes().iter().skip_while(|byte| **byte == 0).count()
    };
    length.saturating_add(1 /* Tag */ + length_size /* Length */)
}

fn write_header(dst: &mut WriteCursor<'_>, tag: u8, length: usize) {
    dst.write_u8(tag);
    if length < 0x80 {
        dst.write_u8(u8::try_from(length).expect("short DER length fits into u8"));
    } else {
        let bytes = length.to_be_bytes();
        let start = bytes.iter().take_while(|byte| **byte == 0).count();
        dst.write_u8(0x80 | u8::try_from(bytes.len() - start).expect("usize length octet count fits into u8"));
        dst.write_slice(&bytes[start..]);
    }
}
