//! Portable NDR32 bodies and v1 type serialization for [MS-RDPEAR sections 6.1–6.3].
//!
//! These are data codecs, not credential operations.
//! Choose raw bodies or [MS-RPCE section 2.2.6] serialization explicitly; neither includes the DER security-package header.
//! The published profile rejects non-null key referents; the native profile consumes their ignored wire slot without a referent.
//!
//! [MS-RDPEAR sections 6.1–6.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpear/
//! [MS-RPCE section 2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, invalid_field_err};

/// Integer byte order inside an NDR32 body or version-one serialization stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endianness {
    Little,
    Big,
}

/// Explicit wire layout; no profile is detected from untrusted input.
///
/// `Published` preserves the by-value public-IDL codec.
/// `WindowsNative` follows the inspected Windows 11 26100 pointer roots, ignored key slot, and NTLM IDs; private operations remain unsupported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NdrProfile {
    #[default]
    Published,
    WindowsNative,
}

impl NdrProfile {
    fn wire_call_id(self, id: u16) -> u16 {
        if self == Self::WindowsNative && (0x201..=0x204).contains(&id) {
            id + 1
        } else {
            id
        }
    }

    fn logical_call_id(self, id: u16) -> Option<u16> {
        if self == Self::WindowsNative {
            match id {
                0x201 => return None,
                0x202..=0x205 => return Some(id - 1),
                _ => {}
            }
        }
        Some(id)
    }
}

/// Independent wire, cumulative allocation, and cumulative element limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NdrLimits {
    pub max_message_size: usize,
    pub max_allocation: usize,
    pub max_elements: usize,
}

impl Default for NdrLimits {
    fn default() -> Self {
        Self {
            max_message_size: 1024 * 1024,
            max_allocation: 4 * 1024 * 1024,
            max_elements: 65536,
        }
    }
}

/// A secret-free diagnostic with a byte offset into the supplied body or serialization stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NdrError {
    pub offset: usize,
    pub reason: &'static str,
}

impl fmt::Display for NdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.reason, self.offset)
    }
}

impl core::error::Error for NdrError {}

pub type NdrResult<T> = Result<T, NdrError>;

struct Writer {
    bytes: Option<Vec<u8>>,
    pos: usize,
    endian: Endianness,
    profile: NdrProfile,
    limits: NdrLimits,
    next_id: u32,
    elements: usize,
}

impl Writer {
    fn fail<T>(&self, reason: &'static str) -> NdrResult<T> {
        Err(NdrError {
            offset: self.pos,
            reason,
        })
    }

    fn put(&mut self, bytes: &[u8]) -> NdrResult<()> {
        let Some(end) = self.pos.checked_add(bytes.len()) else {
            return self.fail("NDR length overflow");
        };
        if self.limits.max_message_size < end || self.limits.max_allocation < end {
            return self.fail("NDR encoding exceeds limit");
        }
        if let Some(output) = &mut self.bytes {
            output.try_reserve_exact(bytes.len()).map_err(|_| NdrError {
                offset: self.pos,
                reason: "NDR allocation failed",
            })?;
            output.extend_from_slice(bytes);
        }
        self.pos = end;
        Ok(())
    }

    fn align(&mut self, alignment: usize) -> NdrResult<()> {
        self.put(&[0; 7][..(alignment - self.pos % alignment) % alignment])
    }

    fn pointer(&mut self, present: bool) -> NdrResult<()> {
        let id = if present {
            let id = self.next_id;
            self.next_id = self.next_id.checked_add(1).ok_or(NdrError {
                offset: self.pos,
                reason: "NDR referent overflow",
            })?;
            id
        } else {
            0
        };
        id.flat(self)
    }

    fn count(&mut self, count: usize) -> NdrResult<()> {
        let Some(elements) = self.elements.checked_add(count) else {
            return self.fail("NDR element overflow");
        };
        if self.limits.max_elements < elements {
            return self.fail("NDR encoding exceeds element limit");
        }
        self.elements = elements;
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    endian: Endianness,
    profile: NdrProfile,
    limits: NdrLimits,
    allocation: usize,
    elements: usize,
}

impl<'a> Reader<'a> {
    fn fail<T>(&self, reason: &'static str) -> NdrResult<T> {
        Err(NdrError {
            offset: self.pos,
            reason,
        })
    }

    fn take(&mut self, count: usize) -> NdrResult<&'a [u8]> {
        let Some(end) = self.pos.checked_add(count) else {
            return self.fail("NDR length overflow");
        };
        let Some(bytes) = self.bytes.get(self.pos..end) else {
            return self.fail("truncated NDR object");
        };
        self.pos = end;
        Ok(bytes)
    }

    fn align(&mut self, alignment: usize) -> NdrResult<()> {
        // NDR interior padding is unspecified; only its extent matters.
        self.take((alignment - self.pos % alignment) % alignment)?;
        Ok(())
    }

    fn reserve<T>(&mut self, count: usize, min_wire_size: usize) -> NdrResult<Vec<T>> {
        let Some(wire_size) = count.checked_mul(min_wire_size) else {
            return self.fail("NDR count overflow");
        };
        if self.bytes.len() - self.pos < wire_size {
            return self.fail("NDR count exceeds remaining object");
        }
        let Some(allocation) = count
            .checked_mul(size_of::<T>())
            .and_then(|n| self.allocation.checked_add(n))
        else {
            return self.fail("NDR allocation overflow");
        };
        let Some(elements) = self.elements.checked_add(count) else {
            return self.fail("NDR element overflow");
        };
        if self.limits.max_allocation < allocation || self.limits.max_elements < elements {
            return self.fail("NDR decoding exceeds limit");
        }
        let mut vector = Vec::new();
        vector.try_reserve_exact(count).map_err(|_| NdrError {
            offset: self.pos,
            reason: "NDR allocation failed",
        })?;
        self.allocation = allocation;
        self.elements = elements;
        Ok(vector)
    }
}

trait Ndr: Sized + Default {
    fn flat(&self, w: &mut Writer) -> NdrResult<()>;
    fn deferred(&self, _writer: &mut Writer) -> NdrResult<()> {
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self>;
    fn read_deferred(&mut self, _reader: &mut Reader<'_>) -> NdrResult<()> {
        Ok(())
    }
}

macro_rules! scalar {
    ($ty:ty, $width:literal) => {
        impl Ndr for $ty {
            fn flat(&self, w: &mut Writer) -> NdrResult<()> {
                w.align($width)?;
                w.put(&match w.endian {
                    Endianness::Little => self.to_le_bytes(),
                    Endianness::Big => self.to_be_bytes(),
                })
            }
            fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
                r.align($width)?;
                let mut bytes = [0; $width];
                bytes.copy_from_slice(r.take($width)?);
                Ok(match r.endian {
                    Endianness::Little => Self::from_le_bytes(bytes),
                    Endianness::Big => Self::from_be_bytes(bytes),
                })
            }
        }
    };
}

scalar!(u8, 1);
scalar!(u16, 2);
scalar!(i16, 2);
scalar!(u32, 4);
scalar!(i32, 4);
scalar!(i64, 8);

impl<T: Ndr> Ndr for Option<T> {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        w.pointer(self.is_some())
    }
    fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
        if let Some(value) = self {
            value.flat(w)?;
            value.deferred(w)?;
        }
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        Ok((u32::read_flat(r)? != 0).then(T::default))
    }
    fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
        if let Some(value) = self {
            *value = T::read_flat(r)?;
            value.read_deferred(r)?;
        }
        Ok(())
    }
}

/// Fixed opaque protocol octets, deliberately redacted by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct Opaque<const N: usize>(pub [u8; N]);

pub type NtChallenge = Opaque<8>;
pub type NtResponse = Opaque<24>;
pub type Lm3Response = Opaque<24>;
pub type UserSessionKey = Opaque<16>;
pub type LmSessionKey = Opaque<8>;
pub type CredentialKey = Opaque<20>;
pub type KeyAgreementHandle = i64;
pub type KerberosError = i32;

impl<const N: usize> Default for Opaque<N> {
    fn default() -> Self {
        Self([0; N])
    }
}
impl<const N: usize> fmt::Debug for Opaque<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Opaque<{N}>(redacted)")
    }
}
impl<const N: usize> Ndr for Opaque<N> {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        w.put(&self.0)
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        let mut bytes = [0; N];
        bytes.copy_from_slice(r.take(N)?);
        Ok(Self(bytes))
    }
}

/// Correlated byte count and unique buffer pointer.
/// Null and non-null empty buffers are distinct; a null pointer may have a nonzero count.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OctetString {
    pub length: u32,
    pub value: Option<Vec<u8>>,
}

pub type CryptoApiBlob = OctetString;

impl fmt::Debug for OctetString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OctetString")
            .field("length", &self.length)
            .field("present", &self.value.is_some())
            .finish()
    }
}
impl Ndr for OctetString {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        self.length.flat(w)?;
        w.pointer(self.value.is_some())
    }
    fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
        if let Some(bytes) = &self.value {
            if usize::try_from(self.length).ok() != Some(bytes.len()) {
                return w.fail("NDR buffer correlation mismatch");
            }
            w.count(bytes.len())?;
            self.length.flat(w)?;
            w.put(bytes)?;
        }
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        let length = u32::read_flat(r)?;
        Ok(Self {
            length,
            value: (u32::read_flat(r)? != 0).then(Vec::new),
        })
    }
    fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
        if let Some(value) = &mut self.value {
            let count = u32::read_flat(r)?;
            if count != self.length {
                return r.fail("NDR buffer correlation mismatch");
            }
            let count = usize::try_from(count).map_err(|_| NdrError {
                offset: r.pos,
                reason: "NDR count overflow",
            })?;
            *value = r.reserve(count, 1)?;
            value.extend_from_slice(r.take(count)?);
        }
        Ok(())
    }
}

/// Counted UTF-16 from [MS-DTYP section 2.3.10], without implicit NUL termination.
///
/// Lengths are bytes; `buffer` contains exactly `length / 2` code units.
/// Unpaired surrogates are retained, not converted to a Rust string.
///
/// [MS-DTYP section 2.3.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-dtyp/
#[derive(Clone, Default, PartialEq, Eq)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: Option<Vec<u16>>,
}
impl fmt::Debug for UnicodeString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnicodeString")
            .field("length", &self.length)
            .field("maximum_length", &self.maximum_length)
            .field("present", &self.buffer.is_some())
            .finish()
    }
}
impl Ndr for UnicodeString {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        w.align(4)?;
        if !self.length.is_multiple_of(2) || self.maximum_length < self.length {
            return w.fail("invalid NDR Unicode byte lengths");
        }
        self.length.flat(w)?;
        self.maximum_length.flat(w)?;
        w.pointer(self.buffer.is_some())
    }
    fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
        if let Some(buffer) = &self.buffer {
            if usize::from(self.length / 2) != buffer.len() {
                return w.fail("NDR Unicode correlation mismatch");
            }
            w.count(buffer.len())?;
            u32::from(self.maximum_length / 2).flat(w)?;
            0u32.flat(w)?;
            u32::from(self.length / 2).flat(w)?;
            for unit in buffer {
                unit.flat(w)?;
            }
        }
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        r.align(4)?;
        let length = u16::read_flat(r)?;
        let maximum_length = u16::read_flat(r)?;
        if !length.is_multiple_of(2) || maximum_length < length {
            return r.fail("invalid NDR Unicode byte lengths");
        }
        Ok(Self {
            length,
            maximum_length,
            buffer: (u32::read_flat(r)? != 0).then(Vec::new),
        })
    }
    fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
        if let Some(buffer) = &mut self.buffer {
            let max = u32::read_flat(r)?;
            let offset = u32::read_flat(r)?;
            let count = u32::read_flat(r)?;
            if max != u32::from(self.maximum_length / 2) || offset != 0 || count != u32::from(self.length / 2) {
                return r.fail("NDR Unicode correlation mismatch");
            }
            let count = usize::try_from(count).map_err(|_| NdrError {
                offset: r.pos,
                reason: "NDR count overflow",
            })?;
            *buffer = r.reserve(count, 2)?;
            for _ in 0..count {
                buffer.push(u16::read_flat(r)?);
            }
        }
        Ok(())
    }
}

macro_rules! structure {
    ($name:ident, $alignment:literal, { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Clone, Default, PartialEq, Eq)]
        pub struct $name { $(pub $field: $ty,)* }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($name), "(redacted)")) }
        }
        impl Ndr for $name {
            fn flat(&self, w: &mut Writer) -> NdrResult<()> {
                w.align($alignment)?;
                $(self.$field.flat(w)?;)*
                Ok(())
            }
            fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
                $(self.$field.deferred(w)?;)*
                Ok(())
            }
            fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
                r.align($alignment)?;
                Ok(Self { $($field: <$ty>::read_flat(r)?,)* })
            }
            fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
                $(self.$field.read_deferred(r)?;)*
                Ok(())
            }
        }
    };
}

/// The key's unused host-pointer member.
///
/// The published profile encodes a null unique-pointer slot and rejects non-null referents.
/// The native profile zeroes its four-byte FC_IGNORE slot and ignores that slot when decoding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReservedPointer;
impl Ndr for ReservedPointer {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        // FC_IGNORE occupies a four-byte wire slot without transmitting a referent.
        0u32.flat(w)
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        let slot = u32::read_flat(r)?;
        if r.profile == NdrProfile::Published && slot != 0 {
            return r.fail("unsupported non-null encryption-key void referent");
        }
        Ok(Self)
    }
}

structure!(Asn1Data, 4, { pdu: u32, data: OctetString });
structure!(EncryptionKey, 4, { reserved1: ReservedPointer, reserved2: i32, reserved3: OctetString });
structure!(PreauthData, 4, { preauth_data_type: i32, preauth_data: OctetString });
structure!(SupplementalCredential, 4, { package_name: UnicodeString, credentials: OctetString });
structure!(EncryptedSecrets, 4, {
    reserved1: u8, reserved2: u8, reserved3: u8, reserved4: CredentialKeyType,
    reserved5: Opaque<20>, reserved6: OctetString
});

/// The enum16 credential-key discriminant, including reserved public-IDL members.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CredentialKeyType {
    #[default]
    Invalid,
    Ium,
    DomainUser,
    LocalUser,
    ExternallySupplied,
}
impl Ndr for CredentialKeyType {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        let value: u16 = match self {
            Self::Invalid => 0,
            Self::Ium => 1,
            Self::DomainUser => 2,
            Self::LocalUser => 3,
            Self::ExternallySupplied => 4,
        };
        value.flat(w)
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        match u16::read_flat(r)? {
            0 => Ok(Self::Invalid),
            1 => Ok(Self::Ium),
            2 => Ok(Self::DomainUser),
            3 => Ok(Self::LocalUser),
            4 => Ok(Self::ExternallySupplied),
            _ => r.fail("invalid NDR credential-key discriminant"),
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct InternalName {
    pub name_type: i16,
    pub name_count: u16,
    pub names: Option<Vec<UnicodeString>>,
}
impl fmt::Debug for InternalName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InternalName(redacted)")
    }
}
impl Ndr for InternalName {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        w.align(4)?;
        self.name_type.flat(w)?;
        self.name_count.flat(w)?;
        w.pointer(self.names.is_some())
    }
    fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
        if let Some(names) = &self.names {
            if usize::from(self.name_count) != names.len() {
                return w.fail("NDR name correlation mismatch");
            }
            w.count(names.len())?;
            u32::from(self.name_count).flat(w)?;
            for name in names {
                name.flat(w)?;
            }
            for name in names {
                name.deferred(w)?;
            }
        }
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        r.align(4)?;
        Ok(Self {
            name_type: i16::read_flat(r)?,
            name_count: u16::read_flat(r)?,
            names: (u32::read_flat(r)? != 0).then(Vec::new),
        })
    }
    fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
        if let Some(names) = &mut self.names {
            let count = u32::read_flat(r)?;
            if count != u32::from(self.name_count) {
                return r.fail("NDR name correlation mismatch");
            }
            let count = usize::from(self.name_count);
            *names = r.reserve(count, 8)?;
            for _ in 0..count {
                names.push(UnicodeString::read_flat(r)?);
            }
            for name in names {
                name.read_deferred(r)?;
            }
        }
        Ok(())
    }
}

/// A conformant structure; the maximum count precedes `CredentialCount`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SupplementalCredentials {
    pub credentials: Vec<SupplementalCredential>,
}
impl fmt::Debug for SupplementalCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SupplementalCredentials(redacted)")
    }
}
impl Ndr for SupplementalCredentials {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        let count = u32::try_from(self.credentials.len()).map_err(|_| NdrError {
            offset: w.pos,
            reason: "NDR count overflow",
        })?;
        w.count(self.credentials.len())?;
        count.flat(w)?;
        count.flat(w)?;
        for credential in &self.credentials {
            credential.flat(w)?;
        }
        Ok(())
    }
    fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
        for credential in &self.credentials {
            credential.deferred(w)?;
        }
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        let count = u32::read_flat(r)?;
        if u32::read_flat(r)? != count {
            return r.fail("NDR credential correlation mismatch");
        }
        let count = usize::try_from(count).map_err(|_| NdrError {
            offset: r.pos,
            reason: "NDR count overflow",
        })?;
        let mut credentials = r.reserve(count, 16)?;
        for _ in 0..count {
            credentials.push(SupplementalCredential::read_flat(r)?);
        }
        Ok(Self { credentials })
    }
    fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
        for credential in &mut self.credentials {
            credential.read_deferred(r)?;
        }
        Ok(())
    }
}

/// A unique length pointer and a unique pointer to a unique conformant byte buffer.
/// All three pointer-presence states are retained independently.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OutputBuffer {
    pub length: Option<u32>,
    pub value: Option<Option<Vec<u8>>>,
}
impl fmt::Debug for OutputBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputBuffer")
            .field("length", &self.length)
            .field("outer_present", &self.value.is_some())
            .finish()
    }
}
impl Ndr for OutputBuffer {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        self.length.flat(w)?;
        w.pointer(self.value.is_some())
    }
    fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
        self.length.deferred(w)?;
        if let Some(value) = &self.value {
            w.pointer(value.is_some())?;
            if let Some(bytes) = value {
                let Some(length) = self.length else {
                    return w.fail("missing NDR referenced length");
                };
                if usize::try_from(length).ok() != Some(bytes.len()) {
                    return w.fail("NDR buffer correlation mismatch");
                }
                w.count(bytes.len())?;
                length.flat(w)?;
                w.put(bytes)?;
            }
        }
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        Ok(Self {
            length: Option::<u32>::read_flat(r)?,
            value: (u32::read_flat(r)? != 0).then_some(None),
        })
    }
    fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
        self.length.read_deferred(r)?;
        if let Some(value) = &mut self.value {
            if u32::read_flat(r)? != 0 {
                let Some(length) = self.length else {
                    return r.fail("missing NDR referenced length");
                };
                if u32::read_flat(r)? != length {
                    return r.fail("NDR buffer correlation mismatch");
                }
                let length = usize::try_from(length).map_err(|_| NdrError {
                    offset: r.pos,
                    reason: "NDR count overflow",
                })?;
                let mut bytes = r.reserve(length, 1)?;
                bytes.extend_from_slice(r.take(length)?);
                *value = Some(bytes);
            }
        }
        Ok(())
    }
}

/// NTLM's 16-bit correlated response length.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct NtlmResponseBuffer {
    pub length: u16,
    pub value: Option<Vec<u8>>,
}
impl fmt::Debug for NtlmResponseBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NtlmResponseBuffer(redacted)")
    }
}
impl Ndr for NtlmResponseBuffer {
    fn flat(&self, w: &mut Writer) -> NdrResult<()> {
        w.align(4)?;
        self.length.flat(w)?;
        w.pointer(self.value.is_some())
    }
    fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
        if let Some(bytes) = &self.value {
            if usize::from(self.length) != bytes.len() {
                return w.fail("NDR buffer correlation mismatch");
            }
            w.count(bytes.len())?;
            u32::from(self.length).flat(w)?;
            w.put(bytes)?;
        }
        Ok(())
    }
    fn read_flat(r: &mut Reader<'_>) -> NdrResult<Self> {
        r.align(4)?;
        Ok(Self {
            length: u16::read_flat(r)?,
            value: (u32::read_flat(r)? != 0).then(Vec::new),
        })
    }
    fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
        let mut value = OctetString {
            length: u32::from(self.length),
            value: self.value.take(),
        };
        value.read_deferred(r)?;
        self.value = value.value;
        Ok(())
    }
}

structure!(NegotiateVersion, 4, { version: u32 });
structure!(BuildAsReqAuthenticatorRequest, 4, {
    encryption_key: Option<EncryptionKey>, armor_key: Option<EncryptionKey>, time_skew: Option<i64>
});
structure!(VerifyServiceTicketRequest, 4, {
    packed_ticket: Option<Asn1Data>, service_key: Option<EncryptionKey>, time_skew: Option<i64>
});
structure!(CreateApReqAuthenticatorRequest, 4, {
    encryption_key: Option<EncryptionKey>, sequence_number: u32,
    client_name: Option<InternalName>, client_realm: Option<UnicodeString>, skew_time: Option<i64>,
    sub_key: Option<EncryptionKey>, auth_data: Option<Asn1Data>, gss_checksum: Option<Asn1Data>, key_usage: u32
});
structure!(DecryptApReplyRequest, 4, { encrypted_reply: Option<Asn1Data>, key: Option<EncryptionKey> });
structure!(UnpackKdcReplyBodyRequest, 4, {
    encrypted_data: Option<Asn1Data>, key: Option<EncryptionKey>, strengthen_key: Option<EncryptionKey>, pdu: u32, key_usage: u32
});
structure!(ComputeTgsChecksumRequest, 4, { request_body: Option<Asn1Data>, key: Option<EncryptionKey>, checksum_type: u32 });
structure!(BuildEncryptedAuthDataRequest, 4, { key_usage: u32, key: Option<EncryptionKey>, plain_auth_data: Option<Asn1Data> });
structure!(PackApReplyRequest, 4, { reply: Option<Asn1Data>, reply_body: Option<Asn1Data>, session_key: Option<EncryptionKey> });
structure!(HashS4UPreauthRequest, 4, { s4u_preauth: Option<Asn1Data>, key: Option<EncryptionKey>, checksum_type: i32 });
structure!(SignS4UPreauthDataRequest, 4, { key: Option<EncryptionKey>, is_request: i32, user_id: Option<Asn1Data>, checksum_type: Option<i32> });
structure!(VerifyChecksumRequest, 4, { key: Option<EncryptionKey>, checksum_type: u32, expected_checksum: OctetString, data_to_check: OctetString });
structure!(DecryptPacCredentialsRequest, 4, { key: Option<EncryptionKey>, version: u32, encryption_type: u32, data: OctetString });
structure!(CreateEcdhKeyAgreementRequest, 4, { key_bit_len: u32 });
structure!(Ignored, 1, { ignored: u8 });
structure!(KeyAgreementRequest, 8, { key_agreement_handle: i64 });
structure!(FinalizeKeyAgreementRequest, 4, { key_agreement_handle: Option<i64>, kerb_etype: u32, remote_nonce: OctetString, x509_public_key: OctetString });
structure!(BuildAsReqAuthenticatorResponse, 4, { preauth_data_type: i32, preauth_data: OctetString });
structure!(VerifyServiceTicketResponse, 4, { decrypted_ticket: Asn1Data, kerb_protocol_error: i32 });
structure!(CreateApReqAuthenticatorResponse, 8, { authenticator_time: i64, authenticator: Asn1Data, kerb_protocol_error: i32 });
structure!(UnpackKdcReplyBodyResponse, 4, { kerb_protocol_error: i32, reply_body: Asn1Data });
structure!(SignS4UPreauthDataResponse, 4, { checksum_type: Option<i32>, checksum: OutputBuffer });
structure!(BoolResponse, 4, { value: i32 });
structure!(DecryptPacCredentialsResponse, 4, { credentials: Option<SupplementalCredentials> });
structure!(CreateEcdhKeyAgreementResponse, 4, { key_agreement_handle: Option<i64>, kerb_err: Option<i32>, encoded_pub_key: OutputBuffer });
structure!(CreateDhKeyAgreementResponse, 4, {
    modulus_p: Option<OctetString>, generator_g: Option<OctetString>, factor_q: Option<OctetString>,
    key_agreement_handle: Option<i64>, kerb_err: Option<i32>, little_endian_public_key: OutputBuffer
});
structure!(FinalizeKeyAgreementResponse, 4, { shared_key: Option<EncryptionKey> });
structure!(Lm20GetNtlm3ChallengeResponseRequest, 4, {
    credential: Option<EncryptedSecrets>, user_name: Option<UnicodeString>, logon_domain_name: Option<UnicodeString>,
    server_name: Option<UnicodeString>, challenge_to_client: Opaque<8>
});
structure!(CalculateNtResponseRequest, 4, { nt_challenge: Option<Opaque<8>>, credential: Option<EncryptedSecrets> });
structure!(CalculateUserSessionKeyNtRequest, 4, { nt_response: Option<Opaque<24>>, credential: Option<EncryptedSecrets> });
structure!(CompareCredentialsRequest, 4, { lhs_credential: Option<EncryptedSecrets>, rhs_credential: Option<EncryptedSecrets> });
structure!(Lm20GetNtlm3ChallengeResponseResponse, 4, {
    ntlm3_response: NtlmResponseBuffer, lm3_response: Opaque<24>, user_session_key: Opaque<16>, lm_session_key: Opaque<8>
});
structure!(CompareCredentialsResponse, 4, { are_nt_owfs_equal: i32, are_lm_owfs_equal: i32, are_sha_owfs_equal: i32 });

macro_rules! call_union {
    ($name:ident { $($variant:ident($ty:ty) = $id:literal),* $(,)? }) => {
        #[derive(Clone, PartialEq, Eq)]
        pub enum $name { $($variant($ty),)* }
        impl $name {
            pub fn call_id(&self) -> u16 { match self { $(Self::$variant(_) => $id,)* } }
            /// Returns the discriminant for the explicitly selected wire profile.
            pub fn wire_call_id(&self, profile: NdrProfile) -> u16 { profile.wire_call_id(self.call_id()) }
            fn flat(&self, w: &mut Writer) -> NdrResult<()> {
                self.wire_call_id(w.profile).flat(w)?;
                match self { $(Self::$variant(value) => value.flat(w),)* }
            }
            fn deferred(&self, w: &mut Writer) -> NdrResult<()> {
                match self { $(Self::$variant(value) => value.deferred(w),)* }
            }
            fn read_flat(r: &mut Reader<'_>, call_id: u16) -> NdrResult<Self> {
                if u16::read_flat(r)? != call_id { return r.fail("NDR union discriminant mismatch") }
                let Some(call_id) = r.profile.logical_call_id(call_id) else {
                    return r.fail("unsupported private NTLM operation");
                };
                match call_id { $($id => Ok(Self::$variant(<$ty>::read_flat(r)?)),)* _ => r.fail("unsupported RDPEAR call ID") }
            }
            fn read_deferred(&mut self, r: &mut Reader<'_>) -> NdrResult<()> {
                match self { $(Self::$variant(value) => value.read_deferred(r),)* }
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).field("call_id", &self.call_id()).finish_non_exhaustive()
            }
        }
    };
}

call_union!(KerberosRequest {
    NegotiateVersion(NegotiateVersion) = 0x100,
    BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest) = 0x101,
    VerifyServiceTicket(VerifyServiceTicketRequest) = 0x102,
    CreateApReqAuthenticator(CreateApReqAuthenticatorRequest) = 0x103,
    DecryptApReply(DecryptApReplyRequest) = 0x104,
    UnpackKdcReplyBody(UnpackKdcReplyBodyRequest) = 0x105,
    ComputeTgsChecksum(ComputeTgsChecksumRequest) = 0x106,
    BuildEncryptedAuthData(BuildEncryptedAuthDataRequest) = 0x107,
    PackApReply(PackApReplyRequest) = 0x108,
    HashS4UPreauth(HashS4UPreauthRequest) = 0x109,
    SignS4UPreauthData(SignS4UPreauthDataRequest) = 0x10a,
    VerifyChecksum(VerifyChecksumRequest) = 0x10b,
    DecryptPacCredentials(DecryptPacCredentialsRequest) = 0x113,
    CreateEcdhKeyAgreement(CreateEcdhKeyAgreementRequest) = 0x114,
    CreateDhKeyAgreement(Ignored) = 0x115,
    DestroyKeyAgreement(KeyAgreementRequest) = 0x116,
    KeyAgreementGenerateNonce(KeyAgreementRequest) = 0x117,
    FinalizeKeyAgreement(FinalizeKeyAgreementRequest) = 0x118,
});
call_union!(KerberosResponseCall {
    NegotiateVersion(NegotiateVersion) = 0x100,
    BuildAsReqAuthenticator(BuildAsReqAuthenticatorResponse) = 0x101,
    VerifyServiceTicket(VerifyServiceTicketResponse) = 0x102,
    CreateApReqAuthenticator(CreateApReqAuthenticatorResponse) = 0x103,
    DecryptApReply(Asn1Data) = 0x104,
    UnpackKdcReplyBody(UnpackKdcReplyBodyResponse) = 0x105,
    ComputeTgsChecksum(Asn1Data) = 0x106,
    BuildEncryptedAuthData(Asn1Data) = 0x107,
    PackApReply(OctetString) = 0x108,
    HashS4UPreauth(OutputBuffer) = 0x109,
    SignS4UPreauthData(SignS4UPreauthDataResponse) = 0x10a,
    VerifyChecksum(BoolResponse) = 0x10b,
    DecryptPacCredentials(DecryptPacCredentialsResponse) = 0x113,
    CreateEcdhKeyAgreement(CreateEcdhKeyAgreementResponse) = 0x114,
    CreateDhKeyAgreement(CreateDhKeyAgreementResponse) = 0x115,
    DestroyKeyAgreement(Ignored) = 0x116,
    KeyAgreementGenerateNonce(OutputBuffer) = 0x117,
    FinalizeKeyAgreement(FinalizeKeyAgreementResponse) = 0x118,
});
call_union!(NtlmRequest {
    NegotiateVersion(NegotiateVersion) = 0x200,
    Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseRequest) = 0x201,
    CalculateNtResponse(CalculateNtResponseRequest) = 0x202,
    CalculateUserSessionKeyNt(CalculateUserSessionKeyNtRequest) = 0x203,
    CompareCredentials(CompareCredentialsRequest) = 0x204,
});
call_union!(NtlmResponseCall {
    NegotiateVersion(NegotiateVersion) = 0x200,
    Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseResponse) = 0x201,
    CalculateNtResponse(Opaque<24>) = 0x202,
    CalculateUserSessionKeyNt(Opaque<16>) = 0x203,
    CompareCredentials(CompareCredentialsResponse) = 0x204,
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KerberosResponse {
    pub status: i32,
    pub call: KerberosResponseCall,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtlmResponse {
    pub status: i32,
    pub call: NtlmResponseCall,
}

trait Object: Sized {
    const ALIGNMENT: usize;
    fn write(&self, w: &mut Writer) -> NdrResult<()>;
    fn read(r: &mut Reader<'_>) -> NdrResult<Self>;
}
macro_rules! request_object {
    ($name:ident, $alignment:literal) => {
        impl Object for $name {
            const ALIGNMENT: usize = $alignment;
            fn write(&self, w: &mut Writer) -> NdrResult<()> {
                self.wire_call_id(w.profile).flat(w)?;
                self.flat(w)?;
                self.deferred(w)
            }
            fn read(r: &mut Reader<'_>) -> NdrResult<Self> {
                let id = u16::read_flat(r)?;
                let mut value = Self::read_flat(r, id)?;
                value.read_deferred(r)?;
                Ok(value)
            }
        }
    };
}
macro_rules! response_object {
    ($name:ident, $call:ident, $alignment:literal) => {
        impl Object for $name {
            const ALIGNMENT: usize = $alignment;
            fn write(&self, w: &mut Writer) -> NdrResult<()> {
                self.call.wire_call_id(w.profile).flat(w)?;
                self.status.flat(w)?;
                self.call.flat(w)?;
                self.call.deferred(w)
            }
            fn read(r: &mut Reader<'_>) -> NdrResult<Self> {
                let id = u16::read_flat(r)?;
                let status = i32::read_flat(r)?;
                let mut call = $call::read_flat(r, id)?;
                call.read_deferred(r)?;
                Ok(Self { status, call })
            }
        }
    };
}
request_object!(KerberosRequest, 8);
request_object!(NtlmRequest, 4);
response_object!(KerberosResponse, KerberosResponseCall, 8);
response_object!(NtlmResponse, NtlmResponseCall, 4);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Framing {
    Body,
    SerializationV1,
}

fn serialize(
    value: &dyn ObjectWrite,
    endian: Endianness,
    limits: NdrLimits,
    framing: Framing,
    profile: NdrProfile,
    count_only: bool,
) -> NdrResult<Writer> {
    let bytes = if count_only {
        None
    } else {
        let length = serialize(value, endian, limits, framing, profile, true)?.pos;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| NdrError {
            offset: 0,
            reason: "NDR allocation failed",
        })?;
        Some(bytes)
    };
    let mut w = Writer {
        bytes,
        pos: 0,
        endian,
        profile,
        limits,
        next_id: 1,
        elements: 0,
    };
    if framing == Framing::SerializationV1 {
        w.put(&[
            1,
            if endian == Endianness::Little { 0x10 } else { 0 },
            8,
            0,
            0xcc,
            0xcc,
            0xcc,
            0xcc,
        ])?;
        w.put(&[0; 8])?;
    }
    // INVARIANT: the body starts at an offset divisible by eight in either framing.
    if profile == NdrProfile::WindowsNative {
        w.pointer(true)?;
        w.align(value.alignment())?;
    }
    value.write_object(&mut w)?;
    if framing == Framing::SerializationV1 {
        w.align(8)?;
        let length = u32::try_from(w.pos - 16).map_err(|_| NdrError {
            offset: w.pos,
            reason: "NDR length overflow",
        })?;
        if let Some(bytes) = &mut w.bytes {
            bytes[8..12].copy_from_slice(&match endian {
                Endianness::Little => length.to_le_bytes(),
                Endianness::Big => length.to_be_bytes(),
            });
        }
    }
    Ok(w)
}

trait ObjectWrite {
    fn alignment(&self) -> usize;
    fn write_object(&self, w: &mut Writer) -> NdrResult<()>;
}
impl<T: Object> ObjectWrite for T {
    fn alignment(&self) -> usize {
        T::ALIGNMENT
    }
    fn write_object(&self, w: &mut Writer) -> NdrResult<()> {
        self.write(w)
    }
}

fn deserialize<T: Object>(
    bytes: &[u8],
    endian: Endianness,
    limits: NdrLimits,
    framing: Framing,
    profile: NdrProfile,
) -> NdrResult<T> {
    let mut r = Reader {
        bytes,
        pos: 0,
        endian,
        profile,
        limits,
        allocation: 0,
        elements: 0,
    };
    if limits.max_message_size < bytes.len() {
        return r.fail("NDR message exceeds limit");
    }
    if framing == Framing::SerializationV1 {
        let header = r.take(8)?;
        if header[0] != 1 || header[2..4] != [8, 0] {
            return r.fail("unsupported NDR serialization header");
        }
        r.endian = match header[1] {
            0x10 => Endianness::Little,
            0 => Endianness::Big,
            _ => return r.fail("invalid NDR endianness"),
        };
        let length = u32::read_flat(&mut r)?;
        r.take(4)?;
        let length = usize::try_from(length).map_err(|_| NdrError {
            offset: r.pos,
            reason: "NDR length overflow",
        })?;
        if !length.is_multiple_of(8) || bytes.len() - 16 != length {
            return r.fail("invalid NDR private object length");
        }
    }
    if profile == NdrProfile::WindowsNative {
        if u32::read_flat(&mut r)? == 0 {
            return r.fail("native NDR call root must not be null");
        }
        r.align(T::ALIGNMENT)?;
    }
    let value = T::read(&mut r)?;
    let padding = if framing == Framing::SerializationV1 {
        (8 - r.pos % 8) % 8
    } else {
        0
    };
    if r.bytes.len() - r.pos != padding {
        return r.fail("trailing NDR object data");
    }
    r.take(padding)?;
    Ok(value)
}

macro_rules! codec {
    ($name:ident) => {
        impl $name {
            /// Serializes one complete NDR32 v1 object, including its common and private headers.
            /// The DER security-package header is separate and must not be supplied here.
            pub fn encode_with_limits(&self, endian: Endianness, limits: NdrLimits) -> NdrResult<Vec<u8>> {
                self.encode_with_profile(endian, limits, NdrProfile::Published)
            }
            /// Decodes exactly one NDR32 v1 object, including its headers and trailing padding.
            pub fn decode_with_limits(bytes: &[u8], limits: NdrLimits) -> NdrResult<Self> {
                Self::decode_with_profile(bytes, limits, NdrProfile::Published)
            }
            /// Serializes a complete v1 object using an explicit package wire profile.
            pub fn encode_with_profile(&self, endian: Endianness, limits: NdrLimits, profile: NdrProfile) -> NdrResult<Vec<u8>> {
                let output = serialize(self, endian, limits, Framing::SerializationV1, profile, false)?;
                output.bytes.ok_or(NdrError { offset: output.pos, reason: "NDR output buffer missing" })
            }
            /// Decodes exactly one complete v1 object without profile detection or fallback.
            pub fn decode_with_profile(bytes: &[u8], limits: NdrLimits, profile: NdrProfile) -> NdrResult<Self> {
                deserialize(bytes, Endianness::Little, limits, Framing::SerializationV1, profile)
            }
            /// Encodes one raw NDR32 body with alignment relative to its first byte.
            /// No serialization headers, serialization trailing padding, or DER security-package header are added.
            /// Limits cover the body bytes, output allocation, and cumulative elements.
            pub fn encode_body_with_limits(&self, endian: Endianness, limits: NdrLimits) -> NdrResult<Vec<u8>> {
                self.encode_body_with_profile(endian, limits, NdrProfile::Published)
            }
            /// Decodes one raw NDR32 body with alignment relative to its first byte.
            /// Requires complete consumption; even serialization trailing padding is rejected.
            /// Byte order and framing are caller choices, with no envelope detection or fallback.
            /// Limits cover the body bytes, referent allocation, and cumulative elements.
            pub fn decode_body_with_limits(bytes: &[u8], endian: Endianness, limits: NdrLimits) -> NdrResult<Self> {
                Self::decode_body_with_profile(bytes, endian, limits, NdrProfile::Published)
            }
            /// Encodes a raw body using the explicit profile's root and key-member representation.
            pub fn encode_body_with_profile(&self, endian: Endianness, limits: NdrLimits, profile: NdrProfile) -> NdrResult<Vec<u8>> {
                let output = serialize(self, endian, limits, Framing::Body, profile, false)?;
                output.bytes.ok_or(NdrError { offset: output.pos, reason: "NDR output buffer missing" })
            }
            /// Decodes a raw body using an explicit profile and requires complete consumption.
            pub fn decode_body_with_profile(bytes: &[u8], endian: Endianness, limits: NdrLimits, profile: NdrProfile) -> NdrResult<Self> {
                deserialize(bytes, endian, limits, Framing::Body, profile)
            }
        }
        impl Encode for $name {
            fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
                let bytes = self.encode_with_limits(Endianness::Little, NdrLimits::default())
                    .map_err(|_| invalid_field_err!("NDR", "invalid or unsupported NDR object", in: dst))?;
                ironrdp_core::ensure_size!(in: dst, size: bytes.len());
                dst.write_slice(&bytes);
                Ok(())
            }
            fn name(&self) -> &'static str { stringify!($name) }
            fn size(&self) -> usize {
                serialize(self, Endianness::Little, NdrLimits::default(), Framing::SerializationV1, NdrProfile::Published, true).map_or(0, |w| w.pos)
            }
        }
        impl<'de> Decode<'de> for $name {
            fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
                let value = Self::decode_with_limits(src.remaining(), NdrLimits::default())
                    .map_err(|error| {
                        src.advance(error.offset.min(src.len()));
                        invalid_field_err!("NDR", error.reason, in: src)
                    })?;
                src.read_remaining();
                Ok(value)
            }
        }
    };
}
codec!(KerberosRequest);
codec!(KerberosResponse);
codec!(NtlmRequest);
codec!(NtlmResponse);
