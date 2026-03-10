//! Length-prefixed Unicode string fields.
//!
//! The two axes — length prefix type and null terminator policy — are encoded as
//! zero-sized marker types, with the actual encode/decode logic driven by sealed traits.
//! Concrete type aliases are provided for every field shape that appears in the RDP specs.

use alloc::borrow::Cow;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

use ironrdp_core::{
    DecodeOwned, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size,
    invalid_field_err,
};

use crate::InvalidUtf16;
use crate::repr::StringRepr;

// ── Sealed trait machinery ────────────────────────────────────────────────────

mod sealed {
    pub trait Sealed {}
}

// ── Length prefix markers ─────────────────────────────────────────────────────

/// Marker: `u16` WCHAR count prefix (`cch` fields, e.g. `cchPCB` in MS-RDPEPS).
pub struct CchU16;
/// Marker: `u32` WCHAR count prefix (`cch` fields, e.g. `cchDeviceInstanceId` in MS-RDPEUSB).
pub struct CchU32;
/// Marker: `u16` byte count prefix (`cb` fields, e.g. `cbDomain` in MS-RDPBCGR).
pub struct CbU16;
/// Marker: `u32` byte count prefix.
pub struct CbU32;

impl sealed::Sealed for CchU16 {}
impl sealed::Sealed for CchU32 {}
impl sealed::Sealed for CbU16 {}
impl sealed::Sealed for CbU32 {}

/// Sealed trait implemented by length-prefix marker types.
///
/// This trait is sealed: only the marker types in this crate ([`CchU16`], [`CchU32`],
/// [`CbU16`], [`CbU32`]) implement it, and no external implementation is possible.
/// It is `pub` so callers can write generic code bounded on it.
pub trait LengthPrefix: sealed::Sealed {
    #[doc(hidden)]
    const WIRE_SIZE: usize;

    #[doc(hidden)]
    const IS_BYTE_COUNT: bool;

    #[doc(hidden)]
    fn read_raw(src: &mut ReadCursor<'_>) -> DecodeResult<usize>;

    #[doc(hidden)]
    fn write_raw(value: usize, dst: &mut WriteCursor<'_>) -> EncodeResult<()>;
}

impl LengthPrefix for CchU16 {
    const WIRE_SIZE: usize = 2;

    const IS_BYTE_COUNT: bool = false;

    fn read_raw(src: &mut ReadCursor<'_>) -> DecodeResult<usize> {
        Ok(usize::from(src.read_u16()))
    }

    fn write_raw(value: usize, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let v: u16 = cast_length!("length prefix", value)?;
        dst.write_u16(v);
        Ok(())
    }
}

impl LengthPrefix for CchU32 {
    const WIRE_SIZE: usize = 4;

    const IS_BYTE_COUNT: bool = false;

    fn read_raw(src: &mut ReadCursor<'_>) -> DecodeResult<usize> {
        Ok(src.read_u32() as usize)
    }

    fn write_raw(value: usize, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let v: u32 = cast_length!("length prefix", value)?;
        dst.write_u32(v);
        Ok(())
    }
}

impl LengthPrefix for CbU16 {
    const WIRE_SIZE: usize = 2;

    const IS_BYTE_COUNT: bool = true;

    fn read_raw(src: &mut ReadCursor<'_>) -> DecodeResult<usize> {
        Ok(usize::from(src.read_u16()))
    }

    fn write_raw(value: usize, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let v: u16 = cast_length!("length prefix", value)?;
        dst.write_u16(v);
        Ok(())
    }
}

impl LengthPrefix for CbU32 {
    const WIRE_SIZE: usize = 4;

    const IS_BYTE_COUNT: bool = true;

    fn read_raw(src: &mut ReadCursor<'_>) -> DecodeResult<usize> {
        Ok(src.read_u32() as usize)
    }

    fn write_raw(value: usize, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let v: u32 = cast_length!("length prefix", value)?;
        dst.write_u32(v);
        Ok(())
    }
}

// ── Null terminator markers ───────────────────────────────────────────────────

/// Marker: null terminator is present on the wire **and** counted in the length prefix.
///
/// Used for: `cchPCB`/`wszPCB` ([MS-RDPEPS] §2.2.1.2), `cchDeviceInstanceId` ([MS-RDPEUSB] §2.2.4.2),
/// `cbClientAddress`, `cbClientDir` ([MS-RDPBCGR] §2.2.1.11.1.1).
///
/// [MS-RDPEPS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeps/
/// [MS-RDPEUSB]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub struct NullCounted;

/// Marker: null terminator is present on the wire but **not** counted in the length prefix.
///
/// Used for: `cbDomain`, `cbUserName`, `cbPassword`, `cbAlternateShell`, `cbWorkingDir`
/// ([MS-RDPBCGR] §2.2.1.11.1.1). Spec: "excludes the length of the mandatory null terminator."
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub struct NullUncounted;

/// Marker: no null terminator on the wire at all.
///
/// Used for: `UNICODE_STRING.String` ([MS-RDPERP] §2.2.1.2.1),
/// `dynamicDSTTimeZoneKeyName` ([MS-RDPBCGR] §2.2.1.11.1.1).
/// Spec: "a non-null-terminated Unicode character string."
///
/// [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp/
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub struct NoNull;

impl sealed::Sealed for NullCounted {}
impl sealed::Sealed for NullUncounted {}
impl sealed::Sealed for NoNull {}

/// Sealed trait implemented by null-terminator policy marker types.
///
/// This trait is sealed: only the marker types in this crate ([`NullCounted`],
/// [`NullUncounted`], [`NoNull`]) implement it. It is `pub` so callers can write
/// generic code bounded on it.
pub trait NullTerminatorPolicy: sealed::Sealed {
    #[doc(hidden)]
    const HAS_NULL_ON_WIRE: bool;

    #[doc(hidden)]
    const NULL_COUNTED_IN_PREFIX: bool;
}

impl NullTerminatorPolicy for NullCounted {
    const HAS_NULL_ON_WIRE: bool = true;
    const NULL_COUNTED_IN_PREFIX: bool = true;
}

impl NullTerminatorPolicy for NullUncounted {
    const HAS_NULL_ON_WIRE: bool = true;
    const NULL_COUNTED_IN_PREFIX: bool = false;
}

impl NullTerminatorPolicy for NoNull {
    const HAS_NULL_ON_WIRE: bool = false;
    const NULL_COUNTED_IN_PREFIX: bool = false;
}

// ── UnicodeStringField ────────────────────────────────────────────────────────

/// A variable-length UTF-16LE string with a self-describing length prefix.
///
/// The two type parameters encode the wire format:
/// - `Prefix`: one of [`CchU16`], [`CchU32`], [`CbU16`], [`CbU32`].
/// - `Null`: one of [`NullCounted`], [`NullUncounted`], [`NoNull`].
///
/// Use the provided type aliases ([`CchPrefixedString`], [`CbPrefixedStringNullExcluded`], etc.)
/// rather than naming this type directly.
pub struct UnicodeStringField<Prefix, Null>(StringRepr, PhantomData<(Prefix, Null)>);

impl<P, N> UnicodeStringField<P, N> {
    /// Creates a `UnicodeStringField` from a native Rust string.
    pub fn new(s: impl Into<String>) -> Self {
        Self(StringRepr::from_native(s.into()), PhantomData)
    }

    /// Creates a `UnicodeStringField` from raw UTF-16LE wire bytes.
    ///
    /// Returns `None` if `bytes` has odd length. This is a convenience wrapper around
    /// [`utf16le_bytes_to_units`] + [`from_wire_units`].
    ///
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    /// [`from_wire_units`]: UnicodeStringField::from_wire_units
    pub fn from_utf16le_bytes(bytes: &[u8]) -> Option<Self> {
        crate::utf16le_bytes_to_units(bytes).map(Self::from_wire_units)
    }

    /// Creates a `UnicodeStringField` from pre-parsed UTF-16 code units.
    ///
    /// The units must not include a null terminator (the null is a wire-level concern
    /// encoded by the `N` type parameter during [`Encode`]). This is the low-level
    /// counterpart to [`decode_owned`] for callers that already have units from
    /// [`utf16le_bytes_to_units`].
    ///
    /// [`Encode`]: ironrdp_core::Encode
    /// [`decode_owned`]: ironrdp_core::DecodeOwned::decode_owned
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    pub fn from_wire_units(units: Vec<u16>) -> Self {
        Self(StringRepr::from_wire_units(units), PhantomData)
    }

    /// Tries to return the string content as a Rust `str`.
    ///
    /// Returns [`InvalidUtf16`] if the wire data contains a lone surrogate.
    /// For strings decoded from the wire, this allocates a new `String`.
    /// For strings constructed from native Rust code, this is a zero-cost borrow.
    pub fn to_native(&self) -> Result<Cow<'_, str>, InvalidUtf16> {
        self.0.to_native()
    }

    /// Returns the string content, replacing any lone surrogates with U+FFFD.
    ///
    /// For strings decoded from the wire, this allocates a new `String`.
    /// For strings constructed from native Rust code, this is a zero-cost borrow.
    pub fn to_native_lossy(&self) -> Cow<'_, str> {
        self.0.to_native_lossy()
    }

    /// Returns the number of UTF-16 code units (WCHARs) in this string.
    ///
    /// O(1) for wire-decoded strings, O(n) for natively-constructed strings.
    pub fn utf16_len(&self) -> usize {
        self.0.utf16_len()
    }

    /// Consumes `self` and returns a validated native `String`.
    ///
    /// Zero-cost when the value was constructed from a native Rust string.
    /// Validates and allocates when the value was decoded from the wire.
    /// Returns [`InvalidUtf16`] if the wire data contains a lone surrogate.
    pub fn into_native(self) -> Result<String, InvalidUtf16> {
        self.0.into_native()
    }

    /// Returns the UTF-16 code units of this string.
    ///
    /// For wire-decoded strings, this is a zero-cost borrow of the stored units.
    /// For strings constructed from native Rust code, this encodes and allocates.
    /// The returned units do not include a null terminator or length prefix.
    pub fn to_wire_units(&self) -> Cow<'_, [u16]> {
        self.0.to_wire_units()
    }

    /// Consumes `self` and returns the UTF-16 code units of this string.
    ///
    /// Zero-cost when the value was decoded from the wire (moves the internal buffer).
    /// Encodes and allocates when the value was constructed from a native string.
    /// The returned units do not include a null terminator or length prefix.
    pub fn into_wire_units(self) -> Vec<u16> {
        self.0.into_wire_units()
    }

    /// Consumes `self` and returns the raw UTF-16LE bytes of the string content.
    ///
    /// Zero-cost when the value was decoded from the wire (moves the internal buffer).
    /// Encodes to UTF-16LE and allocates when the value was constructed from a native string.
    /// The returned bytes do not include a null terminator or length prefix.
    pub fn into_wire(self) -> Vec<u8> {
        self.0.into_wire()
    }
}

impl<P, N> From<String> for UnicodeStringField<P, N> {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl<P, N> From<&str> for UnicodeStringField<P, N> {
    fn from(s: &str) -> Self {
        Self::new(s.to_owned())
    }
}

impl<P, N> TryFrom<UnicodeStringField<P, N>> for String {
    type Error = InvalidUtf16;

    fn try_from(f: UnicodeStringField<P, N>) -> Result<Self, Self::Error> {
        f.0.to_native().map(Cow::into_owned)
    }
}

impl<P, N> fmt::Display for UnicodeStringField<P, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_native_lossy(), f)
    }
}

impl<P, N> fmt::Debug for UnicodeStringField<P, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnicodeStringField({:?})", self.0)
    }
}

impl<P, N> Clone for UnicodeStringField<P, N> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<P, N> PartialEq for UnicodeStringField<P, N> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<P, N> Eq for UnicodeStringField<P, N> {}

// ── Encode ────────────────────────────────────────────────────────────────────

impl<P: LengthPrefix, N: NullTerminatorPolicy> Encode for UnicodeStringField<P, N> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let content_cch = self.0.utf16_len();

        // The prefix value counts either code units or bytes, with or without the null.
        let counted_cch = if N::NULL_COUNTED_IN_PREFIX {
            content_cch + 1 // null is counted
        } else {
            content_cch
        };
        let prefix_value = if P::IS_BYTE_COUNT { counted_cch * 2 } else { counted_cch };

        P::write_raw(prefix_value, dst)?;

        let wire_bytes = self.0.as_wire_bytes();
        dst.write_slice(&wire_bytes);

        if N::HAS_NULL_ON_WIRE {
            dst.write_u16(0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "UnicodeStringField"
    }

    fn size(&self) -> usize {
        P::WIRE_SIZE // length prefix
            + self.0.utf16_byte_len() // content
            + if N::HAS_NULL_ON_WIRE { 2 } else { 0 } // null terminator
    }
}

// ── DecodeOwned ───────────────────────────────────────────────────────────────

impl<P: LengthPrefix, N: NullTerminatorPolicy> DecodeOwned for UnicodeStringField<P, N> {
    fn decode_owned(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Step 1: Read the raw prefix value.
        ensure_size!(in: src, size: P::WIRE_SIZE);
        let raw = P::read_raw(src)?;

        // Step 2: Convert the raw prefix to a code-unit count on the wire.
        let cch_on_wire = if P::IS_BYTE_COUNT {
            if raw % 2 != 0 {
                return Err(invalid_field_err!(
                    "length prefix",
                    "odd byte count for utf-16 string field"
                ));
            }
            raw / 2
        } else {
            raw
        };

        // Step 3: Determine content length (code units of actual string content, excluding null).
        let content_cch = if N::NULL_COUNTED_IN_PREFIX && cch_on_wire > 0 {
            cch_on_wire - 1
        } else {
            // NullUncounted: cch_on_wire is the content length (null follows separately)
            // NoNull: cch_on_wire is the content length
            // NullCounted with cch_on_wire == 0: treat as empty (no null to read)
            cch_on_wire
        };

        // Step 4: Read content code units (bulk copy, convert LE bytes to u16 values).
        ensure_size!(in: src, size: content_cch * 2);
        let slice = src.read_slice(content_cch * 2);
        let units = crate::repr::le_bytes_to_units(slice);

        // Step 5: Consume null terminator if present on wire.
        //
        // NullCounted: the null is the (cch_on_wire)th code unit, already counted in the prefix.
        //   We read it only if cch_on_wire > 0.
        // NullUncounted: the null always follows the content (even for empty content).
        if N::HAS_NULL_ON_WIRE {
            let should_read_null = if N::NULL_COUNTED_IN_PREFIX {
                // NullCounted: only read null if cch_on_wire > 0 (i.e. null was counted)
                cch_on_wire > 0
            } else {
                // NullUncounted: always present on wire regardless of content length
                true
            };
            if should_read_null {
                ensure_size!(in: src, size: 2);
                let _ = src.read_u16(); // null terminator — consume and discard
            }
        }

        Ok(Self(StringRepr::from_wire_units(units), PhantomData))
    }
}

// ── Type aliases ──────────────────────────────────────────────────────────────

/// UTF-16 string with a `u16` WCHAR count prefix, null terminator counted in the prefix.
///
/// Used for `cchPCB`/`wszPCB` in the Preconnection Blob.
///
/// Wire layout: `[u16 cch][cch WCHARs including null]`
///
/// [MS-RDPEPS] §2.2.1.2
///
/// [MS-RDPEPS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeps/
pub type CchPrefixedString = UnicodeStringField<CchU16, NullCounted>;

/// UTF-16 string with a `u32` WCHAR count prefix, null terminator counted in the prefix.
///
/// Used for `cchDeviceInstanceId`, `cchContainerId`, `cchHwIds`, `cchCompatIds` in the
/// USB device descriptor.
///
/// Wire layout: `[u32 cch][cch WCHARs including null]`
///
/// [MS-RDPEUSB] §2.2.4.2
///
/// [MS-RDPEUSB]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/
pub type Cch32PrefixedString = UnicodeStringField<CchU32, NullCounted>;

/// UTF-16 string with a `u16` byte count prefix, null terminator **not** counted in the prefix.
///
/// Used for `cbDomain`, `cbUserName`, `cbPassword`, `cbAlternateShell`, `cbWorkingDir`
/// in the Info Packet. Spec: "excludes the length of the mandatory null terminator."
///
/// Wire layout: `[u16 cb][cb/2 WCHARs][null WCHAR]`
///
/// [MS-RDPBCGR] §2.2.1.11.1.1
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub type CbPrefixedStringNullExcluded = UnicodeStringField<CbU16, NullUncounted>;

/// UTF-16 string with a `u16` byte count prefix, null terminator counted in the prefix.
///
/// Used for `cbClientAddress`, `cbClientDir` in the Extended Info Packet.
/// Spec: "includes the length of the mandatory null terminator."
///
/// Wire layout: `[u16 cb][cb/2 WCHARs including null]`
///
/// [MS-RDPBCGR] §2.2.1.11.1.1
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub type CbPrefixedStringNullIncluded = UnicodeStringField<CbU16, NullCounted>;

/// Non-null-terminated UTF-16 string with a `u16` byte count prefix.
///
/// Used for `UNICODE_STRING.String` in Remote Programs (RAIL).
/// Spec: "A non-null-terminated Unicode character string."
///
/// Wire layout: `[u16 cb][cb/2 WCHARs]`
///
/// [MS-RDPERP] §2.2.1.2.1
///
/// [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp/
pub type RailUnicodeString = UnicodeStringField<CbU16, NoNull>;

/// Non-null-terminated UTF-16 string with a `u16` byte count prefix.
///
/// Used for `dynamicDSTTimeZoneKeyName`. Spec: "A variable-length array of Unicode
/// characters with no terminating null character."
///
/// Wire layout: `[u16 cb][cb/2 WCHARs]`
///
/// [MS-RDPBCGR] §2.2.1.11.1.1
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub type CbPrefixedStringNoNull = UnicodeStringField<CbU16, NoNull>;
