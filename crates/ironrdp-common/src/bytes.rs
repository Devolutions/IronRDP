use core::borrow::Borrow;
use core::fmt;
use core::ops::Deref;

#[derive(Clone, PartialEq, Eq)]
pub struct Bytes<'a>(BytesInner<'a>);

impl Bytes<'_> {
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.deref()
    }
}

impl<'a> From<&'a [u8]> for Bytes<'a> {
    #[inline]
    fn from(value: &'a [u8]) -> Self {
        Self::from_slice(value)
    }
}

impl Deref for Bytes<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        self.0.deref()
    }
}

impl AsRef<[u8]> for Bytes<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl Borrow<[u8]> for Bytes<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl fmt::Debug for Bytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes(")?;
        write!(f, "0x")?;
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02X}"))?;
        write!(f, ")")
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Bytes<'a> {
    #[inline]
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let bytes = <&[u8]>::arbitrary(u)?;
        Ok(Self::from(bytes))
    }
}

#[allow(unused_imports)]
pub use self::details::*;

#[cfg(not(feature = "alloc"))]
mod details {
    use super::*;

    impl<'a> Bytes<'a> {
        #[inline]
        pub const fn from_slice(bytes: &'a [u8]) -> Self {
            Self(bytes)
        }
    }

    pub(super) type BytesInner<'a> = &'a [u8];
}

#[cfg(feature = "alloc")]
mod details {
    use alloc::borrow::Cow;
    use alloc::vec::Vec;

    use super::*;

    pub(super) type BytesInner<'a> = Cow<'a, [u8]>;

    impl<'a> Bytes<'a> {
        #[inline]
        pub const fn from_slice(bytes: &'a [u8]) -> Self {
            Self(Cow::Borrowed(bytes))
        }

        #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
        #[inline]
        pub fn into_bytes(self) -> Vec<u8> {
            self.0.into_owned()
        }

        #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
        #[inline]
        pub fn into_owned(self) -> OwnedBytes {
            Bytes(Cow::Owned(self.0.into_owned()))
        }
    }

    /// Owned version of [`Bytes`]
    #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
    pub type OwnedBytes = Bytes<'static>;

    impl OwnedBytes {
        #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
        #[inline]
        pub const fn from_bytes(bytes: Vec<u8>) -> Self {
            Self(Cow::Owned(bytes))
        }
    }

    #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
    impl From<Vec<u8>> for OwnedBytes {
        #[inline]
        fn from(value: Vec<u8>) -> Self {
            Self::from_bytes(value)
        }
    }
}
