//! Mouse Cursor Virtual Channel Extension PDUs defined by [MS-RDPEMSC].
//!
//! The pointer shape structures share their layout with [MS-RDPBCGR] `TS_POINTERATTRIBUTE` and
//! `TS_FP_LARGEPOINTERATTRIBUTE`, so they are encoded and decoded through the `ironrdp-pdu`
//! codecs for those structures.
//!
//! [MS-RDPEMSC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/

use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err, unexpected_message_type_err,
};
use ironrdp_dvc::DvcEncode;
use ironrdp_pdu::pointer::{
    CachedPointerAttribute, ColorPointerAttribute, LargePointerAttribute, Point16, PointerAttribute, PointerUpdateData,
};

const PDUTYPE_CS_CAPS_ADVERTISE: u8 = 0x01;
const PDUTYPE_SC_CAPS_CONFIRM: u8 = 0x02;
const PDUTYPE_SC_MOUSEPTR_UPDATE: u8 = 0x03;

const TS_UPDATETYPE_MOUSEPTR_SYSTEM_NULL: u8 = 0x05;
const TS_UPDATETYPE_MOUSEPTR_SYSTEM_DEFAULT: u8 = 0x06;
const TS_UPDATETYPE_MOUSEPTR_POSITION: u8 = 0x08;
const TS_UPDATETYPE_MOUSEPTR_CACHED: u8 = 0x0A;
const TS_UPDATETYPE_MOUSEPTR_POINTER: u8 = 0x0B;
const TS_UPDATETYPE_MOUSEPTR_LARGE_POINTER: u8 = 0x0C;

const HEADER_SIZE: usize = 1 /* pduType */ + 1 /* updateType */ + 2 /* reserved */;
const CACHED_POINTER_INDEX_SIZE: usize = 2 /* cachedPointerIndex */;

/// `signature` of every capability set from [2.2.2.2] RDP_MOUSE_CURSOR_CAPSET, the bytes `CAPS`.
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
pub const CAPSET_SIGNATURE: u32 = 0x5350_4143;

/// `RDP_MOUSE_CURSOR_CAPVERSION_1` from [2.2.2.2] RDP_MOUSE_CURSOR_CAPSET.
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
pub const CAPVERSION_1: u32 = 0x0000_0001;

/// A capability set as described in [2.2.2.2] RDP_MOUSE_CURSOR_CAPSET.
///
/// Version 1 ([2.2.2.3.1] RDP_MOUSE_CURSOR_CAPSET_VERSION1) carries no capability data.
/// Other versions keep their data so that a peer can skip them.
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
/// [2.2.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySet {
    pub version: u32,
    pub data: Vec<u8>,
}

impl CapabilitySet {
    const NAME: &'static str = "RDP_MOUSE_CURSOR_CAPSET";
    const FIXED_PART_SIZE: usize = 4 /* signature */ + 4 /* version */ + 4 /* size */;

    /// The version 1 capability set.
    pub const fn version1() -> Self {
        Self {
            version: CAPVERSION_1,
            data: Vec::new(),
        }
    }
}

impl Encode for CapabilitySet {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        if self.version == CAPVERSION_1 && !self.data.is_empty() {
            return Err(invalid_field_err!(
                Self::NAME,
                "data",
                "version 1 capability set has no data"
            ));
        }

        dst.write_u32(CAPSET_SIGNATURE);
        dst.write_u32(self.version);
        dst.write_u32(cast_length!("size", self.size(), in: dst)?);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }
}

impl Decode<'_> for CapabilitySet {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let signature = src.read_u32();
        if signature != CAPSET_SIGNATURE {
            return Err(invalid_field_err!("signature", "invalid capability set signature", in: src));
        }
        let version = src.read_u32();
        let size: usize = cast_length!("size", src.read_u32(), in: src)?;
        let Some(data_size) = size.checked_sub(Self::FIXED_PART_SIZE) else {
            return Err(invalid_field_err!("size", "capability set is smaller than its header", in: src));
        };
        if version == CAPVERSION_1 && data_size != 0 {
            return Err(invalid_field_err!("size", "version 1 capability set must be 12 bytes", in: src));
        }
        ensure_size!(in: src, size: data_size);
        let data = src.read_slice(data_size).to_vec();

        Ok(Self { version, data })
    }
}

/// [2.2.3.1] RDP_MOUSE_CURSOR_CAPS_ADVERTISE_PDU, sent by the client.
///
/// [2.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsAdvertisePdu {
    /// Capability sets, each version at most once ([3.3.5.1]).
    ///
    /// [3.3.5.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
    pub caps_sets: Vec<CapabilitySet>,
}

/// [2.2.3.2] RDP_MOUSE_CURSOR_CAPS_CONFIRM_PDU, sent by the server.
///
/// [2.2.3.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsConfirmPdu {
    pub caps_set: CapabilitySet,
}

/// A pointer image from [2.2.2.5] TS_POINTERATTRIBUTE or [2.2.2.6] TS_LARGEPOINTERATTRIBUTE.
///
/// [2.2.2.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
/// [2.2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerShape {
    pub xor_bpp: u16,
    /// Pointer Image Cache slot the image is stored in.
    pub cache_index: u16,
    pub hot_spot: Point16,
    pub width: u16,
    pub height: u16,
    /// Bottom-up XOR mask, each scan line padded to 2 bytes.
    pub xor_mask: Vec<u8>,
    /// Bottom-up 1-bpp AND mask, each scan line padded to 2 bytes.
    pub and_mask: Vec<u8>,
}

impl PointerShape {
    fn as_pointer_attribute(&self) -> PointerAttribute<'_> {
        PointerAttribute {
            xor_bpp: self.xor_bpp,
            color_pointer: ColorPointerAttribute {
                cache_index: self.cache_index,
                hot_spot: self.hot_spot,
                width: self.width,
                height: self.height,
                xor_mask: &self.xor_mask,
                and_mask: &self.and_mask,
            },
        }
    }

    fn as_large_pointer_attribute(&self) -> LargePointerAttribute<'_> {
        LargePointerAttribute {
            xor_bpp: self.xor_bpp,
            cache_index: self.cache_index,
            hot_spot: self.hot_spot,
            width: self.width,
            height: self.height,
            xor_mask: &self.xor_mask,
            and_mask: &self.and_mask,
        }
    }
}

impl From<PointerAttribute<'_>> for PointerShape {
    fn from(pointer: PointerAttribute<'_>) -> Self {
        let color = pointer.color_pointer;
        Self {
            xor_bpp: pointer.xor_bpp,
            cache_index: color.cache_index,
            hot_spot: color.hot_spot,
            width: color.width,
            height: color.height,
            xor_mask: color.xor_mask.to_vec(),
            and_mask: color.and_mask.to_vec(),
        }
    }
}

impl From<LargePointerAttribute<'_>> for PointerShape {
    fn from(pointer: LargePointerAttribute<'_>) -> Self {
        Self {
            xor_bpp: pointer.xor_bpp,
            cache_index: pointer.cache_index,
            hot_spot: pointer.hot_spot,
            width: pointer.width,
            height: pointer.height,
            xor_mask: pointer.xor_mask.to_vec(),
            and_mask: pointer.and_mask.to_vec(),
        }
    }
}

/// [2.2.3.3] RDP_MOUSE_CURSOR_MOUSEPTR_UPDATE_PDU, sent by the server, by `updateType`.
///
/// [2.2.3.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerUpdate {
    /// `TS_UPDATETYPE_MOUSEPTR_SYSTEM_NULL`: hide the pointer.
    Hidden,
    /// `TS_UPDATETYPE_MOUSEPTR_SYSTEM_DEFAULT`: show the operating system default pointer.
    Default,
    /// `TS_UPDATETYPE_MOUSEPTR_POSITION`: move the pointer.
    Position(Point16),
    /// `TS_UPDATETYPE_MOUSEPTR_CACHED`: show a pointer from the Pointer Image Cache.
    Cached { cache_index: u16 },
    /// `TS_UPDATETYPE_MOUSEPTR_POINTER`: cache and show a pointer up to 96x96 pixels.
    Pointer(PointerShape),
    /// `TS_UPDATETYPE_MOUSEPTR_LARGE_POINTER`: cache and show a pointer up to 384x384 pixels.
    LargePointer(PointerShape),
}

impl PointerUpdate {
    /// The equivalent [MS-RDPBCGR] pointer update, for the pointer handling shared with
    /// fast-path and slow-path updates.
    ///
    /// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    pub fn as_pointer_update_data(&self) -> PointerUpdateData<'_> {
        match self {
            Self::Hidden => PointerUpdateData::SetHidden,
            Self::Default => PointerUpdateData::SetDefault,
            Self::Position(position) => PointerUpdateData::SetPosition(*position),
            Self::Cached { cache_index } => PointerUpdateData::Cached(CachedPointerAttribute {
                cache_index: *cache_index,
            }),
            Self::Pointer(shape) => PointerUpdateData::New(shape.as_pointer_attribute()),
            Self::LargePointer(shape) => PointerUpdateData::Large(shape.as_large_pointer_attribute()),
        }
    }

    fn update_type(&self) -> u8 {
        match self {
            Self::Hidden => TS_UPDATETYPE_MOUSEPTR_SYSTEM_NULL,
            Self::Default => TS_UPDATETYPE_MOUSEPTR_SYSTEM_DEFAULT,
            Self::Position(_) => TS_UPDATETYPE_MOUSEPTR_POSITION,
            Self::Cached { .. } => TS_UPDATETYPE_MOUSEPTR_CACHED,
            Self::Pointer(_) => TS_UPDATETYPE_MOUSEPTR_POINTER,
            Self::LargePointer(_) => TS_UPDATETYPE_MOUSEPTR_LARGE_POINTER,
        }
    }

    fn encode_payload(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::Hidden | Self::Default => Ok(()),
            Self::Position(position) => position.encode(dst),
            Self::Cached { cache_index } => CachedPointerAttribute {
                cache_index: *cache_index,
            }
            .encode(dst),
            Self::Pointer(shape) => shape.as_pointer_attribute().encode(dst),
            Self::LargePointer(shape) => shape.as_large_pointer_attribute().encode(dst),
        }
    }

    fn payload_size(&self) -> usize {
        match self {
            Self::Hidden | Self::Default => 0,
            Self::Position(position) => position.size(),
            Self::Cached { .. } => CACHED_POINTER_INDEX_SIZE,
            Self::Pointer(shape) => shape.as_pointer_attribute().size(),
            Self::LargePointer(shape) => shape.as_large_pointer_attribute().size(),
        }
    }

    // The optional trailing pad byte of the pointer attributes is left unread.
    fn decode_payload(update_type: u8, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let update = match update_type {
            TS_UPDATETYPE_MOUSEPTR_SYSTEM_NULL => Self::Hidden,
            TS_UPDATETYPE_MOUSEPTR_SYSTEM_DEFAULT => Self::Default,
            TS_UPDATETYPE_MOUSEPTR_POSITION => Self::Position(Point16::decode(src)?),
            TS_UPDATETYPE_MOUSEPTR_CACHED => Self::Cached {
                cache_index: CachedPointerAttribute::decode(src)?.cache_index,
            },
            TS_UPDATETYPE_MOUSEPTR_POINTER => Self::Pointer(PointerAttribute::decode(src)?.into()),
            TS_UPDATETYPE_MOUSEPTR_LARGE_POINTER => Self::LargePointer(LargePointerAttribute::decode(src)?.into()),
            _ => return Err(invalid_field_err!("updateType", "unsupported pointer update type", in: src)),
        };
        Ok(update)
    }
}

/// A mouse cursor channel message, prefaced by [2.2.2.1] RDP_MOUSE_CURSOR_HEADER.
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MouseCursorPdu {
    CapsAdvertise(CapsAdvertisePdu),
    CapsConfirm(CapsConfirmPdu),
    PointerUpdate(PointerUpdate),
}

impl MouseCursorPdu {
    const NAME: &'static str = "RDP_MOUSE_CURSOR_PDU";
}

impl Encode for MouseCursorPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let (pdu_type, update_type) = match self {
            Self::CapsAdvertise(_) => (PDUTYPE_CS_CAPS_ADVERTISE, 0),
            Self::CapsConfirm(_) => (PDUTYPE_SC_CAPS_CONFIRM, 0),
            Self::PointerUpdate(update) => (PDUTYPE_SC_MOUSEPTR_UPDATE, update.update_type()),
        };
        dst.write_u8(pdu_type);
        dst.write_u8(update_type);
        dst.write_u16(0); // reserved

        match self {
            Self::CapsAdvertise(pdu) => {
                for caps_set in &pdu.caps_sets {
                    caps_set.encode(dst)?;
                }
                Ok(())
            }
            Self::CapsConfirm(pdu) => pdu.caps_set.encode(dst),
            Self::PointerUpdate(update) => update.encode_payload(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let body = match self {
            Self::CapsAdvertise(pdu) => pdu.caps_sets.iter().map(Encode::size).sum(),
            Self::CapsConfirm(pdu) => pdu.caps_set.size(),
            Self::PointerUpdate(update) => update.payload_size(),
        };
        HEADER_SIZE + body
    }
}

impl DvcEncode for MouseCursorPdu {}

impl Decode<'_> for MouseCursorPdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: HEADER_SIZE);

        let pdu_type = src.read_u8();
        let update_type = src.read_u8();
        let _reserved = src.read_u16();

        let pdu = match pdu_type {
            PDUTYPE_CS_CAPS_ADVERTISE => {
                // The capability sets fill the rest of the message.
                let mut caps_sets = Vec::new();
                while !src.is_empty() {
                    caps_sets.push(CapabilitySet::decode(src)?);
                }
                Self::CapsAdvertise(CapsAdvertisePdu { caps_sets })
            }
            PDUTYPE_SC_CAPS_CONFIRM => Self::CapsConfirm(CapsConfirmPdu {
                caps_set: CapabilitySet::decode(src)?,
            }),
            PDUTYPE_SC_MOUSEPTR_UPDATE => Self::PointerUpdate(PointerUpdate::decode_payload(update_type, src)?),
            _ => return Err(unexpected_message_type_err!(pdu_type, in: src)),
        };
        Ok(pdu)
    }
}
