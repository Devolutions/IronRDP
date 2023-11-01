#[cfg(feature = "alloc")]
use alloc::string::String;
use core::marker::PhantomData;
use core::{cmp::Ordering, fmt};

#[macro_export]
macro_rules! FixedStringZ {
    ($max_size:literal, $encoding:path) => {
        FixedStringZ<{ $max_size * <$encoding as $crate::StringEncoding>::CODE_UNIT_LENGTH }, $encoding>
    }
}

#[macro_export]
macro_rules! Utf8FixedStringZ {
    ($max_size:literal) => {
        $crate::FixedStringZ!($max_size, $crate::Utf8Encoding)
    };
}

#[macro_export]
macro_rules! Utf16FixedStringZ {
    ($max_size:literal) => {
        $crate::FixedStringZ!($max_size, $crate::Utf16Encoding)
    };
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum StringEncodingErrorKind {
    ValidUpTo(usize),
    NullTerminatorNotFound,
}

/// Error which can occur when attempting to interpret a sequence of [`u8`] as a string
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct StringEncodingError {
    encoding_name: &'static str,
    kind: StringEncodingErrorKind,
}

impl fmt::Display for StringEncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            StringEncodingErrorKind::ValidUpTo(valid_up_to) => {
                write!(
                    f,
                    "incomplete {} byte sequence from index {}",
                    self.encoding_name, valid_up_to
                )
            }
            StringEncodingErrorKind::NullTerminatorNotFound => {
                write!(f, "null-terminator not found in {} byte sequence", self.encoding_name)
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StringEncodingError {}

/// The string is too big to fit into the given fixed-length string
#[derive(Copy, Eq, PartialEq, Clone, Debug)]
pub struct StringTooBigError {
    size: usize,
    maximum: usize,
}

impl StringTooBigError {
    /// The size of the string too big
    #[must_use]
    #[inline]
    pub const fn size(&self) -> usize {
        self.size
    }

    /// The maximum size for the fixed-length string
    #[must_use]
    #[inline]
    pub const fn maximum(&self) -> usize {
        self.maximum
    }
}

impl fmt::Display for StringTooBigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-byte string is too big to fit into a {}-byte array",
            self.size, self.maximum
        )
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StringTooBigError {}

pub trait StringEncoding: crate::private::Sealed + Sized {
    const NAME: &'static str;
    const CODE_UNIT_LENGTH: usize;

    /// Checks whether a sequence of [`u8`] is a valid string and returns the index of the null terminator if it is.
    fn find_null_terminator(input: &[u8]) -> Result<usize, StringEncodingError>;
}

pub struct Utf8Encoding;

impl crate::private::Sealed for Utf8Encoding {}

impl StringEncoding for Utf8Encoding {
    const NAME: &'static str = "UTF-8";
    const CODE_UNIT_LENGTH: usize = 1;

    fn find_null_terminator(input: &[u8]) -> Result<usize, StringEncodingError> {
        let null_idx = input.iter().copied().position(|i| i == 0).ok_or(StringEncodingError {
            encoding_name: Self::NAME,
            kind: StringEncodingErrorKind::NullTerminatorNotFound,
        })?;

        core::str::from_utf8(&input[..null_idx]).map_err(|e| StringEncodingError {
            encoding_name: Self::NAME,
            kind: StringEncodingErrorKind::ValidUpTo(e.valid_up_to()),
        })?;

        Ok(null_idx)
    }
}

/// Basic Multilingual Plane (BMP), first plane ("plane 0"), of the Unicode standard.
///
/// Unlike UTF-16, BMP encoding is not a variable-length encoding.
///
/// [Wikipedia](https://en.wikipedia.org/wiki/Plane_(Unicode)#Basic_Multilingual_Plane)
pub struct BmpEncoding;

impl crate::private::Sealed for BmpEncoding {}

impl StringEncoding for BmpEncoding {
    const NAME: &'static str = "BMP";
    const CODE_UNIT_LENGTH: usize = 2;

    fn find_null_terminator(input: &[u8]) -> Result<usize, StringEncodingError> {
        let u16_it = input
            .chunks_exact(2)
            .map(|code_unit| u16::from_le_bytes([code_unit[0], code_unit[1]]));

        let mut count = 0;

        for res in char::decode_utf16(u16_it) {
            match res {
                Ok(c) if c == '\0' => return Ok(count * 2),
                Ok(_) => {}
                Err(_) => {
                    return Err(StringEncodingError {
                        encoding_name: Self::NAME,
                        kind: StringEncodingErrorKind::ValidUpTo(count),
                    })
                }
            }

            count += 2;
        }

        Err(StringEncodingError {
            encoding_name: Self::NAME,
            kind: StringEncodingErrorKind::NullTerminatorNotFound,
        })
    }
}

pub struct Utf16Encoding;

impl crate::private::Sealed for Utf16Encoding {}

impl StringEncoding for Utf16Encoding {
    const NAME: &'static str = "UTF-16";
    const CODE_UNIT_LENGTH: usize = 2;

    fn find_null_terminator(input: &[u8]) -> Result<usize, StringEncodingError> {
        let null_idx = input
            .chunks_exact(2)
            .position(|chunk| chunk == [0, 0])
            .ok_or(StringEncodingError {
                encoding_name: Self::NAME,
                kind: StringEncodingErrorKind::NullTerminatorNotFound,
            })?;

        let u16_it = input[..null_idx]
            .chunks_exact(2)
            .map(|code_unit| u16::from_le_bytes([code_unit[0], code_unit[1]]));

        let valid_up_to = char::decode_utf16(u16_it).position(|res| res.is_err());

        if let Some(valid_up_to) = valid_up_to {
            return Err(StringEncodingError {
                encoding_name: Self::NAME,
                kind: StringEncodingErrorKind::ValidUpTo(valid_up_to),
            });
        }

        Ok(null_idx)
    }
}

/// An array of at most 256 UTF-8 characters, null-terminated
pub type Utf8StringZ256 = Utf8FixedStringZ!(256);

/// An array of at most 256 UTF-16 characters, null-terminated
pub type Utf16StringZ256 = Utf16FixedStringZ!(256);

/// Null-terminated, fixed-length string.
///
/// The string may be encoded on at most `N` bytes including the null terminator.
///
/// It’s best for `N` to not be too big (let’s say, no more than `1024`), because this string can
/// actually be stored on the stack. This also increase the size of the struct holding it. You
/// may consider an alternate option if this becomes a concern.
///
/// Note that implementation of traits [`PartialOrd`] and [`PartialEq`] is deliberately ignoring
/// the "leftover bytes" which are found after the null terminator. The leftover bytes may contain garbage
/// and should be ignored for all intents and purposes.
/// However, it’s possible to access the leftover bytes using [`inner_array`](Self::inner_array) or
/// [`leftover_bytes`](Self::leftover_bytes) for inspection purposes, but in general no
/// useful payload should be stored into these.
#[derive(Clone)]
pub struct FixedStringZ<const N: usize, E> {
    // INVARIANT: `self.array[..self.null_idx]` contains a string encoded properly (UTF-8, UTF-16…)
    // INVARIANT: `self.array[self.null_idx..(self.null_idx + CODE_UNIT_LENGTH)]` contains 0s (the last character of the string is a null terminator)
    array: [u8; N],
    // INVARIANT: `self.null_idx <= N - CODE_UNIT_LENGTH`
    null_idx: usize,
    _encoding: PhantomData<E>,
}

impl<const N: usize, E> FixedStringZ<N, E> {
    /// The raw inner array
    #[deprecated(note = "this method may leak leftover bytes")]
    pub const fn inner_array(&self) -> &[u8; N] {
        &self.array
    }

    /// Returns the index of the first byte of the null terminator
    pub const fn null_terminator_index(&self) -> usize {
        self.null_idx
    }

    /// The bytes holding a valid string for the given [`StringEncoding`]
    pub fn string_bytes(&self) -> &[u8] {
        &self.array[..self.null_idx]
    }

    /// Leftover bytes which may contain garbage that should be ignored
    #[deprecated(note = "leftover bytes should be ignored")]
    pub fn leftover_bytes(&self) -> &[u8] {
        &self.array[self.null_idx..]
    }

    /// Compares this string with another, ignoring leftover bytes
    pub fn compare<const OTHER_N: usize>(&self, other: &FixedStringZ<OTHER_N, E>) -> Ordering {
        self.array[..self.null_idx].cmp(&other.array[..other.null_idx])
    }
}

impl<const N: usize, E: StringEncoding> FixedStringZ<N, E> {
    pub const ENCODED_LENGTH: usize = N;

    pub fn from_array(array: [u8; N]) -> Result<Self, StringEncodingError> {
        Ok(Self {
            array,
            null_idx: E::find_null_terminator(&array)?,
            _encoding: PhantomData,
        })
    }

    // TODO: decode and encode methods
}

impl<const N: usize> FixedStringZ<N, Utf8Encoding> {
    pub fn new(input: &str) -> Result<Self, StringTooBigError> {
        if input.len() > N - Utf8Encoding::CODE_UNIT_LENGTH {
            Err(StringTooBigError {
                size: input.len(),
                maximum: N - Utf8Encoding::CODE_UNIT_LENGTH,
            })
        } else {
            Ok(Self::new_truncate(input))
        }
    }

    pub fn new_truncate(input: &str) -> Self {
        let null_idx = core::cmp::min(input.len(), N - Utf8Encoding::CODE_UNIT_LENGTH);

        let mut array = [0; N];
        array.copy_from_slice(&input.as_bytes()[..null_idx]);

        Self {
            array,
            null_idx,
            _encoding: PhantomData,
        }
    }

    pub fn to_str(&self) -> &str {
        // NOTE(perf): If this code ever becomes a bottleneck (which I doubt it will ever because fixed-length strings
        // are not supposed to be very big), we could use `from_utf8_unchecked` assuming all invariants are
        // properly upheld. This means that `Utf8Encoding::find_null_terminator` must be correctly implemented and
        // audited.
        core::str::from_utf8(&self.array[..self.null_idx]).unwrap()
    }
}

impl<const N: usize> FixedStringZ<N, Utf16Encoding> {
    pub fn new(input: &str) -> Result<Self, StringTooBigError> {
        if input.encode_utf16().count() * Utf16Encoding::CODE_UNIT_LENGTH > N - Utf16Encoding::CODE_UNIT_LENGTH {
            Err(StringTooBigError {
                size: input.len(),
                maximum: N - Utf16Encoding::CODE_UNIT_LENGTH,
            })
        } else {
            Ok(Self::new_truncate(input))
        }
    }

    pub fn new_truncate(input: &str) -> Self {
        let mut array = [0; N];
        let mut null_idx = 0;

        // Until N - 2 so we always have a null terminator at the end
        let dst_it = array[..N - 2].chunks_exact_mut(2);

        input.encode_utf16().zip(dst_it).for_each(|(code_unit, dst)| {
            dst.copy_from_slice(&code_unit.to_le_bytes());
            null_idx += 2;
        });

        Self {
            array,
            null_idx,
            _encoding: PhantomData,
        }
    }

    #[cfg(feature = "alloc")]
    #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
    pub fn to_utf8(&self) -> String {
        let u16_it = self.array[..self.null_idx]
            .chunks_exact(2)
            .map(|code_unit| u16::from_le_bytes([code_unit[0], code_unit[1]]));
        char::decode_utf16(u16_it)
            .collect::<Result<_, _>>()
            .expect("valid UTF-16 string until null terminator")
    }
}

impl<const N: usize> FixedStringZ<N, BmpEncoding> {
    #[cfg(feature = "alloc")]
    #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
    pub fn to_utf8(&self) -> String {
        let u16_it = self.array[..self.null_idx]
            .chunks_exact(2)
            .map(|code_unit| u16::from_le_bytes([code_unit[0], code_unit[1]]));
        char::decode_utf16(u16_it)
            .collect::<Result<_, _>>()
            .expect("valid UTF-16 string until null terminator")
    }
}

impl<const N: usize, E> PartialEq for FixedStringZ<N, E> {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other).is_eq()
    }
}

impl<const N: usize, E> Eq for FixedStringZ<N, E> {}

impl<const N: usize, E> PartialOrd for FixedStringZ<N, E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare(other))
    }
}

impl<const N: usize, E> Ord for FixedStringZ<N, E> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl<const N: usize, E: StringEncoding> fmt::Debug for FixedStringZ<N, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FixedStringZ[{}](", E::NAME)?;
        self.array.iter().take(self.null_idx).copied().try_for_each(|byte| {
            if byte.is_ascii() {
                write!(f, "{}", char::from(byte))
            } else {
                write!(f, "\\x{byte:02X}")
            }
        })?;
        write!(f, ")")
    }
}

// TODO: property tests in ironrdp-testsuite-core
