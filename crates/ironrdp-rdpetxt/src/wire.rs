//! Field-level codec shared by the PDU structures.
//!
//! Every MS-RDPETXT structure is a plain sequence of little-endian fields, so each structure
//! encodes, decodes, and sizes itself as the concatenation of its fields.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use ironrdp_core::{
    DecodeOwned as _, DecodeResult, Encode as _, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size,
    invalid_field_err,
};

use crate::pdu::{Guid, TextString};

/// A value with a fixed little-endian wire representation.
pub(crate) trait Wire: Sized {
    fn wire_size(&self) -> usize;

    fn write(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()>;

    fn read(src: &mut ReadCursor<'_>) -> DecodeResult<Self>;
}

macro_rules! impl_wire_int {
    ($($ty:ty => $size:literal, $write:ident, $read:ident;)*) => {
        $(
            impl Wire for $ty {
                fn wire_size(&self) -> usize {
                    $size
                }

                fn write(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
                    ensure_size!(in: dst, size: $size);
                    dst.$write(*self);
                    Ok(())
                }

                fn read(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
                    ensure_size!(in: src, size: $size);
                    Ok(src.$read())
                }
            }
        )*
    };
}

impl_wire_int! {
    u8 => 1, write_u8, read_u8;
    u16 => 2, write_u16, read_u16;
    i16 => 2, write_i16, read_i16;
    u32 => 4, write_u32, read_u32;
    i32 => 4, write_i32, read_i32;
    u64 => 8, write_u64, read_u64;
}

/// A one-byte `BOOLEAN`: any nonzero value is TRUE, and TRUE is written as 1.
impl Wire for bool {
    fn wire_size(&self) -> usize {
        1
    }

    fn write(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        u8::from(*self).write(dst)
    }

    fn read(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(u8::read(src)? != 0)
    }
}

impl Wire for Guid {
    fn wire_size(&self) -> usize {
        16
    }

    fn write(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: 16);
        dst.write_array(self.0);
        Ok(())
    }

    fn read(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 16);
        Ok(Self(src.read_array()))
    }
}

/// A 4-byte count of UTF-16 code units followed by the code units, without a terminator.
impl Wire for TextString {
    fn wire_size(&self) -> usize {
        self.size()
    }

    fn write(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.encode(dst)
    }

    fn read(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Self::decode_owned(src)
    }
}

/// A 4-byte element count followed by the elements.
impl<T: Wire> Wire for Vec<T> {
    fn wire_size(&self) -> usize {
        4 /* count */ + self.iter().map(Wire::wire_size).sum::<usize>()
    }

    fn write(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let count: u32 = cast_length!("count", self.len())?;
        count.write(dst)?;
        self.iter().try_for_each(|element| element.write(dst))
    }

    fn read(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let count = usize::try_from(u32::read(src)?).map_err(|_| invalid_field_err!("count", "too large"))?;
        // Every element takes at least one byte, so a larger count cannot be satisfied.
        if src.len() < count {
            return Err(invalid_field_err!("count", "exceeds the remaining data", in: src));
        }
        core::iter::repeat_with(|| T::read(src)).take(count).collect()
    }
}

/// Declares a structure whose wire form is its fields in declaration order.
macro_rules! wire_struct {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                pub $field:ident: $ty:ty,
            )*
        }
    ) => {
        $(#[$meta])*
        pub struct $name {
            $(
                $(#[$field_meta])*
                pub $field: $ty,
            )*
        }

        impl $crate::wire::Wire for $name {
            fn wire_size(&self) -> usize {
                0 $(+ $crate::wire::Wire::wire_size(&self.$field))*
            }

            fn write(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
                $($crate::wire::Wire::write(&self.$field, dst)?;)*
                // Structures without fields write nothing.
                let _ = dst;
                Ok(())
            }

            fn read(src: &mut ironrdp_core::ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
                // Structures without fields read nothing.
                let _ = &src;
                Ok(Self {
                    $($field: $crate::wire::Wire::read(src)?,)*
                })
            }
        }
    };
}

pub(crate) use wire_struct;

/// Implements [`Wire`] for a newtype over an integer by delegating to the integer.
macro_rules! wire_newtype {
    ($($name:ident($ty:ty);)*) => {
        $(
            impl $crate::wire::Wire for $name {
                fn wire_size(&self) -> usize {
                    $crate::wire::Wire::wire_size(&self.0)
                }

                fn write(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
                    $crate::wire::Wire::write(&self.0, dst)
                }

                fn read(src: &mut ironrdp_core::ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
                    <$ty as $crate::wire::Wire>::read(src).map(Self)
                }
            }
        )*
    };
}

pub(crate) use wire_newtype;
