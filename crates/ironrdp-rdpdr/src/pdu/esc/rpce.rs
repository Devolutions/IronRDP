//! PDUs for [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions] as required by [MS-RDPESC].
//!
//! [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c15

use std::mem::size_of;

use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::CharacterSet;
use ironrdp_pdu::{cast_length, ensure_size, invalid_field_err, DecodeError, DecodeResult, EncodeResult};

/// Wrapper struct for [MS-RPCE] PDUs that allows for common [`Encode`], [`Encode`], and [`Self::decode`] implementations.
///
/// Structs which are meant to be encoded into an [MS-RPCE] message should typically implement [`HeaderlessEncode`],
/// and their `new` function should return a [`Pdu`] wrapping the underlying struct.
///
/// ```rust
/// #[derive(Debug)]
/// pub struct RpceEncodePdu {
///     example_field: u32,
/// }
///
/// impl RpceEncodePdu {
///     /// `new` returns a `Pdu` wrapping the underlying struct.
///     pub fn new(example_field: u32) -> rpce::Pdu<Self> {
///         rpce::Pdu(Self { example_field })
///     }
/// }
///
/// /// The underlying struct should implement `HeaderlessEncode`.
/// impl rpce::HeaderlessEncode for RpceEncodePdu {
///     fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
///         ensure_size!(in: dst, size: self.size());
///         dst.write_u32(self.return_code.into());
///         Ok(())
///     }
///
///     fn name(&self) -> &'static str {
///         "RpceEncodePdu"
///     }
///
///     fn size(&self) -> usize {
///         std::mem::size_of<u32>()
///     }
/// }
/// ```
///
/// See [`super::LongReturn`] for a live example of an encodable PDU.
///
/// Structs which are meant to be decoded from an [MS-RPCE] message should typically implement [`HeaderlessDecode`],
/// and their `decode` function should return a [`Pdu`] wrapping the underlying struct.
///
/// ```rust
/// pub struct RpceDecodePdu {
///     example_field: u32,
/// }
///
/// impl RpceDecodePdu {
///     /// `decode` returns a `Pdu` wrapping the underlying struct.
///     pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<rpce::Pdu<Self>> {
///         Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
///     }
///
///     fn size() -> usize {
///         std::mem::size_of<u32>()
///     }
/// }
///
/// /// The underlying struct should implement `HeaderlessDecode`.
/// impl rpce::HeaderlessDecode for RpceDecodePdu {
///    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self>
///    where
///         Self: Sized,
///    {
///        ensure_size!(in: src, size: Self::size());
///        let example_field = src.read_u32();
///        Ok(Self { example_field })
///     }
/// }
/// ```
///
/// See [`super::EstablishContextCall`] for a live example of a decodable PDU.
#[derive(Debug)]
pub struct Pdu<T>(pub T);

impl<T> Pdu<T> {
    pub fn into_inner(self) -> T {
        self.0
    }

    pub fn into_inner_ref(&self) -> &T {
        &self.0
    }
}

impl<T: HeaderlessDecode> Pdu<T> {
    /// Decodes the instance from a buffer stripping it of its [`StreamHeader`] and [`TypeHeader`].
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Pdu<T>> {
        // We expect `StreamHeader::decode`, `TypeHeader::decode`, and `T::decode` to each
        // call `ensure_size!` to ensure that the buffer is large enough, so we can safely
        // omit that check here.
        let _stream_header = StreamHeader::decode(src)?;
        let _type_header = TypeHeader::decode(src)?;
        let pdu = T::decode(src, charset)?;
        Ok(Self(pdu))
    }
}

impl<T: HeaderlessEncode> ironrdp_pdu::Encode for Pdu<T> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(ctx: self.name(), in: dst, size: self.size());
        let stream_header = StreamHeader::default();
        let type_header = TypeHeader::new(cast_length!("Pdu<T>", "size", self.size())?);

        stream_header.encode(dst)?;
        type_header.encode(dst)?;
        HeaderlessEncode::encode(&self.0, dst)?;

        // Pad response to be 8-byte aligned.
        let padding_size = padding_size(&self.0);
        if padding_size > 0 {
            dst.write_slice(&vec![0; padding_size]);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn size(&self) -> usize {
        StreamHeader::size() + TypeHeader::size() + HeaderlessEncode::size(&self.0) + padding_size(&self.0)
    }
}

impl<T: HeaderlessEncode> Encode for Pdu<T> {}

/// Trait for types that can be encoded into an [MS-RPCE] message.
///
/// Implementers should typically avoid implementing this trait directly
/// and instead implement [`HeaderlessEncode`], and wrap it in a [`Pdu`].
pub trait Encode: ironrdp_pdu::Encode + Send + std::fmt::Debug {}

/// Trait for types that can be encoded into an [MS-RPCE] message.
///
/// Implementers should typically implement this trait instead of [`Encode`].
pub trait HeaderlessEncode: Send + std::fmt::Debug {
    /// Encodes the instance into a buffer sans its [`StreamHeader`] and [`TypeHeader`].
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()>;
    /// Returns the name associated with this RPCE PDU.
    fn name(&self) -> &'static str;
    /// Returns the size of the instance sans its [`StreamHeader`] and [`TypeHeader`].
    fn size(&self) -> usize;
}

/// Trait for types that can be decoded from an [MS-RPCE] message.
///
/// Implementers should typically implement this trait for a given type `T`
/// and then call [`Pdu::decode`] to decode the instance. See [`Pdu`] for more
/// details and an example.
pub trait HeaderlessDecode: Sized {
    /// Decodes the instance from a buffer sans its [`StreamHeader`] and [`TypeHeader`].
    ///
    /// `charset` is an optional parameter that can be used to specify the character set
    /// when relevant. This is useful for accounting for the "A" vs "W" variants of certain
    /// opcodes e.g. [`ListReadersA`][`super::ScardIoCtlCode::ListReadersA`] vs [`ListReadersW`][`super::ScardIoCtlCode::ListReadersW`].
    fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self>;
}

/// [2.2.6.1] Common Type Header for the Serialization Stream
///
/// [2.2.6.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/6d75d40e-e2d2-4420-b9e9-8508a726a9ae
struct StreamHeader {
    version: u8,
    endianness: Endianness,
    common_header_length: u16,
    filler: u32,
}

impl Default for StreamHeader {
    fn default() -> Self {
        Self {
            version: 1,
            endianness: Endianness::LittleEndian,
            common_header_length: 8,
            filler: 0xCCCC_CCCC,
        }
    }
}

impl StreamHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u8(self.version);
        dst.write_u8(self.endianness.into());
        dst.write_u16(self.common_header_length);
        dst.write_u32(self.filler);
        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::size());
        let version = src.read_u8();
        let endianness = Endianness::try_from(src.read_u8())?;
        let common_header_length = src.read_u16();
        let filler = src.read_u32();
        if endianness == Endianness::LittleEndian {
            Ok(Self {
                version,
                endianness,
                common_header_length,
                filler,
            })
        } else {
            Err(invalid_field_err!(
                "decode",
                "StreamHeader",
                "server returned big-endian data, parsing not implemented"
            ))
        }
    }

    fn size() -> usize {
        size_of::<u8>() + size_of::<u8>() + size_of::<u16>() + size_of::<u32>()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
enum Endianness {
    BigEndian = 0x00,
    LittleEndian = 0x10,
}

impl TryFrom<u8> for Endianness {
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Endianness::BigEndian),
            0x10 => Ok(Endianness::LittleEndian),
            _ => Err(invalid_field_err!("try_from", "RpceEndianness", "unsupported value")),
        }
    }
}

impl From<Endianness> for u8 {
    fn from(endianness: Endianness) -> Self {
        endianness as u8
    }
}

/// [2.2.6.2] Private Header for Constructed Type
///
/// [2.2.6.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/63949ba8-bc88-4c0c-9377-23f14b197827
#[derive(Debug)]
struct TypeHeader {
    object_buffer_length: u32,
    filler: u32,
}

impl TypeHeader {
    fn new(object_buffer_length: u32) -> Self {
        Self {
            object_buffer_length,
            filler: 0,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u32(self.object_buffer_length);
        dst.write_u32(self.filler);
        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::size());
        let object_buffer_length = src.read_u32();
        let filler = src.read_u32();

        Ok(Self {
            object_buffer_length,
            filler,
        })
    }
}

impl TypeHeader {
    fn size() -> usize {
        size_of::<u32>() * 2
    }
}

/// Calculates the padding required for an [MS-RPCE] message
/// to be 8-byte aligned.
fn padding_size(pdu: &impl HeaderlessEncode) -> usize {
    let tail = pdu.size() % 8;
    if tail > 0 {
        8 - tail
    } else {
        0
    }
}
