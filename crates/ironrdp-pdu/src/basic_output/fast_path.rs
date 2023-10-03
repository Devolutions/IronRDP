#[cfg(test)]
mod tests;

use bit_field::BitField;
use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::bitmap::BitmapUpdateData;
use super::pointer::PointerUpdateData;
use super::surface_commands::{SurfaceCommand, SURFACE_COMMAND_HEADER_SIZE};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::rdp::client_info::CompressionType;
use crate::rdp::headers::{CompressionFlags, SHARE_DATA_HEADER_COMPRESSION_MASK};
use crate::{decode_cursor, per, PduDecode, PduEncode, PduResult};

/// Implements the Fast-Path RDP message header PDU.
/// TS_FP_UPDATE_PDU
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathHeader {
    pub flags: EncryptionFlags,
    pub data_length: usize,
    forced_long_length: bool,
}

impl FastPathHeader {
    const NAME: &str = "TS_FP_UPDATE_PDU header";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<EncryptionFlags>();

    pub fn new(flags: EncryptionFlags, data_length: usize) -> Self {
        Self {
            flags,
            data_length,
            forced_long_length: false,
        }
    }

    fn minimal_size(&self) -> usize {
        Self::FIXED_PART_SIZE + per::sizeof_length(self.data_length as u16)
    }
}

impl PduEncode for FastPathHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let mut header = 0u8;
        header.set_bits(0..2, 0); // fast-path action
        header.set_bits(6..8, self.flags.bits());
        dst.write_u8(header);

        let length = self.data_length + self.size();
        if length > u16::MAX as usize {
            return Err(invalid_message_err!("length", "fastpath PDU length is too big"));
        }

        if self.forced_long_length {
            // Preserve same layout for header as received
            per::write_long_length(dst, length as u16);
        } else {
            per::write_length(dst, length as u16);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        if self.forced_long_length {
            Self::FIXED_PART_SIZE + per::U16_SIZE
        } else {
            self.minimal_size()
        }
    }
}

impl<'de> PduDecode<'de> for FastPathHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let header = src.read_u8();
        let flags = EncryptionFlags::from_bits_truncate(header.get_bits(6..8));

        let (length, sizeof_length) = per::read_length(src)
            .map_err(|e| invalid_message_err!("length", "Invalid encoded fast path PDU length").with_source(e))?;
        if (length as usize) < sizeof_length + Self::FIXED_PART_SIZE {
            return Err(invalid_message_err!(
                "length",
                "received fastpath PDU length is smaller than header size"
            ));
        }
        let data_length = length as usize - sizeof_length - Self::FIXED_PART_SIZE;
        // Detect case, when received packet has non-optimal packet length packing
        let forced_long_length = per::sizeof_length(length) != sizeof_length;

        Ok(FastPathHeader {
            flags,
            data_length,
            forced_long_length,
        })
    }
}

/// TS_FP_UPDATE
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPathUpdatePdu<'a> {
    pub fragmentation: Fragmentation,
    pub update_code: UpdateCode,
    pub compression_flags: Option<CompressionFlags>,
    // NOTE: always Some when compression flags is Some
    pub compression_type: Option<CompressionType>,
    pub data: &'a [u8],
}

impl FastPathUpdatePdu<'_> {
    const NAME: &'static str = "TS_FP_UPDATE";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u8>();
}

impl PduEncode for FastPathUpdatePdu<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        if self.data.len() > u16::MAX as usize {
            return Err(invalid_message_err!("data", "fastpath PDU data is too big"));
        }

        let mut header = 0u8;
        header.set_bits(0..4, self.update_code.to_u8().unwrap());
        header.set_bits(4..6, self.fragmentation.to_u8().unwrap());

        dst.write_u8(header);

        if self.compression_flags.is_some() {
            header.set_bits(6..8, Compression::COMPRESSION_USED.bits());
            let compression_flags_with_type = self.compression_flags.map(|f| f.bits()).unwrap_or(0)
                | self.compression_type.and_then(|f| f.to_u8()).unwrap_or(0);
            dst.write_u8(compression_flags_with_type);
        }

        dst.write_u16(self.data.len() as u16);
        dst.write_slice(self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let compression_flags_size = if self.compression_flags.is_some() {
            std::mem::size_of::<u8>()
        } else {
            0
        };

        Self::FIXED_PART_SIZE + compression_flags_size + std::mem::size_of::<u16>() + self.data.len()
    }
}

impl<'de> PduDecode<'de> for FastPathUpdatePdu<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let header = src.read_u8();

        let update_code = header.get_bits(0..4);
        let update_code =
            UpdateCode::from_u8(update_code).ok_or(invalid_message_err!("updateHeader", "Invalid update code"))?;

        let fragmentation = header.get_bits(4..6);
        let fragmentation = Fragmentation::from_u8(fragmentation)
            .ok_or(invalid_message_err!("updateHeader", "Invalid fragmentation"))?;

        let compression = Compression::from_bits_truncate(header.get_bits(6..8));

        let (compression_flags, compression_type) = if compression.contains(Compression::COMPRESSION_USED) {
            let expected_size = std::mem::size_of::<u8>() + std::mem::size_of::<u16>();
            ensure_size!(in: src, size: expected_size);

            let compression_flags_with_type = src.read_u8();
            let compression_flags =
                CompressionFlags::from_bits_truncate(compression_flags_with_type & !SHARE_DATA_HEADER_COMPRESSION_MASK);
            let compression_type =
                CompressionType::from_u8(compression_flags_with_type & SHARE_DATA_HEADER_COMPRESSION_MASK)
                    .ok_or_else(|| invalid_message_err!("compressionFlags", "invalid compression type"))?;

            (Some(compression_flags), Some(compression_type))
        } else {
            let expected_size = std::mem::size_of::<u16>();
            ensure_size!(in: src, size: expected_size);

            (None, None)
        };

        let data_length = src.read_u16() as usize;
        ensure_size!(in: src, size: data_length);
        let data = src.read_slice(data_length);

        Ok(Self {
            fragmentation,
            update_code,
            compression_flags,
            compression_type,
            data,
        })
    }
}

/// TS_FP_UPDATE data
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastPathUpdate<'a> {
    SurfaceCommands(Vec<SurfaceCommand<'a>>),
    Bitmap(BitmapUpdateData<'a>),
    Pointer(PointerUpdateData<'a>),
}

impl<'a> FastPathUpdate<'a> {
    const NAME: &'static str = "TS_FP_UPDATE data";

    pub fn decode_with_code(src: &'a [u8], code: UpdateCode) -> PduResult<Self> {
        let mut cursor = ReadCursor::<'a>::new(src);
        Self::decode_cursor_with_code(&mut cursor, code)
    }

    pub fn decode_cursor_with_code(src: &mut ReadCursor<'a>, code: UpdateCode) -> PduResult<Self> {
        match code {
            UpdateCode::SurfaceCommands => {
                let mut commands = Vec::with_capacity(1);
                while src.len() >= SURFACE_COMMAND_HEADER_SIZE {
                    commands.push(decode_cursor::<SurfaceCommand<'_>>(src)?);
                }

                Ok(Self::SurfaceCommands(commands))
            }
            UpdateCode::Bitmap => Ok(Self::Bitmap(decode_cursor(src)?)),
            UpdateCode::HiddenPointer => Ok(Self::Pointer(PointerUpdateData::SetHidden)),
            UpdateCode::DefaultPointer => Ok(Self::Pointer(PointerUpdateData::SetDefault)),
            UpdateCode::PositionPointer => Ok(Self::Pointer(PointerUpdateData::SetPosition(decode_cursor(src)?))),
            UpdateCode::ColorPointer => {
                let color = decode_cursor(src)?;
                Ok(Self::Pointer(PointerUpdateData::Color(color)))
            }
            UpdateCode::CachedPointer => Ok(Self::Pointer(PointerUpdateData::Cached(decode_cursor(src)?))),
            UpdateCode::NewPointer => Ok(Self::Pointer(PointerUpdateData::New(decode_cursor(src)?))),
            UpdateCode::LargePointer => Ok(Self::Pointer(PointerUpdateData::Large(decode_cursor(src)?))),
            _ => Err(invalid_message_err!("updateCode", "Invalid fast path update code")),
        }
    }

    pub fn as_short_name(&self) -> &str {
        match self {
            Self::SurfaceCommands(_) => "Surface Commands",
            Self::Bitmap(_) => "Bitmap",
            Self::Pointer(_) => "Pointer",
        }
    }
}

impl PduEncode for FastPathUpdate<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        match self {
            Self::SurfaceCommands(commands) => {
                for command in commands {
                    command.encode(dst)?;
                }
            }
            Self::Bitmap(bitmap) => {
                bitmap.encode(dst)?;
            }
            Self::Pointer(pointer) => match pointer {
                PointerUpdateData::SetHidden => {}
                PointerUpdateData::SetDefault => {}
                PointerUpdateData::SetPosition(inner) => inner.encode(dst)?,
                PointerUpdateData::Color(inner) => inner.encode(dst)?,
                PointerUpdateData::Cached(inner) => inner.encode(dst)?,
                PointerUpdateData::New(inner) => inner.encode(dst)?,
                PointerUpdateData::Large(inner) => inner.encode(dst)?,
            },
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::SurfaceCommands(commands) => commands.iter().map(|c| c.size()).sum::<usize>(),
            Self::Bitmap(bitmap) => bitmap.size(),
            Self::Pointer(pointer) => match pointer {
                PointerUpdateData::SetHidden => 0,
                PointerUpdateData::SetDefault => 0,
                PointerUpdateData::SetPosition(inner) => inner.size(),
                PointerUpdateData::Color(inner) => inner.size(),
                PointerUpdateData::Cached(inner) => inner.size(),
                PointerUpdateData::New(inner) => inner.size(),
                PointerUpdateData::Large(inner) => inner.size(),
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum UpdateCode {
    Orders = 0x0,
    Bitmap = 0x1,
    Palette = 0x2,
    Synchronize = 0x3,
    SurfaceCommands = 0x4,
    HiddenPointer = 0x5,
    DefaultPointer = 0x6,
    PositionPointer = 0x8,
    ColorPointer = 0x9,
    CachedPointer = 0xa,
    NewPointer = 0xb,
    LargePointer = 0xc,
}

impl From<&FastPathUpdate<'_>> for UpdateCode {
    fn from(update: &FastPathUpdate<'_>) -> Self {
        match update {
            FastPathUpdate::SurfaceCommands(_) => Self::SurfaceCommands,
            FastPathUpdate::Bitmap(_) => Self::Bitmap,
            FastPathUpdate::Pointer(action) => match action {
                PointerUpdateData::SetHidden => Self::HiddenPointer,
                PointerUpdateData::SetDefault => Self::DefaultPointer,
                PointerUpdateData::SetPosition(_) => Self::PositionPointer,
                PointerUpdateData::Color(_) => Self::ColorPointer,
                PointerUpdateData::Cached(_) => Self::CachedPointer,
                PointerUpdateData::New(_) => Self::NewPointer,
                PointerUpdateData::Large(_) => Self::LargePointer,
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum Fragmentation {
    Single = 0x0,
    Last = 0x1,
    First = 0x2,
    Next = 0x3,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EncryptionFlags: u8 {
        const SECURE_CHECKSUM = 0x1;
        const ENCRYPTED = 0x2;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Compression: u8 {
        const COMPRESSION_USED = 0x2;
    }
}
