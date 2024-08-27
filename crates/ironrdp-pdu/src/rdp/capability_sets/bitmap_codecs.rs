#[cfg(test)]
mod tests;

use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::decode;
use crate::{PduDecode, PduEncode, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

const RFX_ICAP_VERSION: u16 = 0x0100;
const RFX_ICAP_TILE_SIZE: u16 = 0x40;
const RFX_ICAP_COLOR_CONVERSION: u8 = 1;
const RFX_ICAP_TRANSFORM_BITS: u8 = 1;
const RFX_ICAP_LENGTH: usize = 8;

const RFX_CAPSET_BLOCK_TYPE: u16 = 0xcbc1;
const RFX_CAPSET_TYPE: u16 = 0xcfc0;
const RFX_CAPSET_STATIC_DATA_LENGTH: usize = 13;

const RFX_CAPS_BLOCK_TYPE: u16 = 0xcbc0;
const RFX_CAPS_BLOCK_LENGTH: u32 = 8;
const RFX_CAPS_NUM_CAPSETS: u16 = 1;
const RFX_CAPS_STATIC_DATA_LENGTH: usize = 8;

const RFX_CLIENT_CAPS_CONTAINER_STATIC_DATA_LENGTH: usize = 12;

const NSCODEC_LENGTH: usize = 3;
const CODEC_STATIC_DATA_LENGTH: usize = 19;

#[rustfmt::skip]
const GUID_NSCODEC: Guid = Guid(0xca8d_1bb9, 0x000f, 0x154f, 0x58, 0x9f, 0xae, 0x2d, 0x1a, 0x87, 0xe2, 0xd6);
#[rustfmt::skip]
const GUID_REMOTEFX: Guid = Guid(0x7677_2f12, 0xbd72, 0x4463, 0xaf, 0xb3, 0xb7, 0x3c, 0x9c, 0x6f, 0x78, 0x86);
#[rustfmt::skip]
const GUID_IMAGE_REMOTEFX: Guid = Guid(0x2744_ccd4, 0x9d8a, 0x4e74, 0x80, 0x3c, 0x0e, 0xcb, 0xee, 0xa1, 0x9c, 0x54);
#[rustfmt::skip]
const GUID_IGNORE: Guid = Guid(0x9c43_51a6, 0x3535, 0x42ae, 0x91, 0x0c, 0xcd, 0xfc, 0xe5, 0x76, 0x0b, 0x58);

#[derive(Debug, PartialEq, Eq)]
pub struct Guid(u32, u16, u16, u8, u8, u8, u8, u8, u8, u8, u8);

impl Guid {
    const NAME: &'static str = "Guid";

    const FIXED_PART_SIZE: usize = 16;
}

impl PduEncode for Guid {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.0);
        dst.write_u16(self.1);
        dst.write_u16(self.2);
        dst.write_u8(self.3);
        dst.write_u8(self.4);
        dst.write_u8(self.5);
        dst.write_u8(self.6);
        dst.write_u8(self.7);
        dst.write_u8(self.8);
        dst.write_u8(self.9);
        dst.write_u8(self.10);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Guid {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let guid1 = src.read_u32();
        let guid2 = src.read_u16();
        let guid3 = src.read_u16();
        let guid4 = src.read_u8();
        let guid5 = src.read_u8();
        let guid6 = src.read_u8();
        let guid7 = src.read_u8();
        let guid8 = src.read_u8();
        let guid9 = src.read_u8();
        let guid10 = src.read_u8();
        let guid11 = src.read_u8();

        Ok(Guid(
            guid1, guid2, guid3, guid4, guid5, guid6, guid7, guid8, guid9, guid10, guid11,
        ))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BitmapCodecs(pub Vec<Codec>);

impl BitmapCodecs {
    const NAME: &'static str = "BitmapCodecs";

    const FIXED_PART_SIZE: usize = 1 /* len */;
}

impl PduEncode for BitmapCodecs {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u8(cast_length!("len", self.0.len())?);

        for codec in self.0.iter() {
            codec.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.0.iter().map(PduEncode::size).sum::<usize>()
    }
}

impl<'de> PduDecode<'de> for BitmapCodecs {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let codecs_count = src.read_u8();

        let mut codecs = Vec::with_capacity(codecs_count as usize);
        for _ in 0..codecs_count {
            codecs.push(Codec::decode(src)?);
        }

        Ok(BitmapCodecs(codecs))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Codec {
    pub id: u8,
    pub property: CodecProperty,
}

impl Codec {
    const NAME: &'static str = "Codec";

    const FIXED_PART_SIZE: usize = CODEC_STATIC_DATA_LENGTH;
}

impl PduEncode for Codec {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let guid = match &self.property {
            CodecProperty::NsCodec(_) => GUID_NSCODEC,
            CodecProperty::RemoteFx(_) => GUID_REMOTEFX,
            CodecProperty::ImageRemoteFx(_) => GUID_IMAGE_REMOTEFX,
            CodecProperty::Ignore => GUID_IGNORE,
            _ => return Err(other_err!("invalid codec")),
        };
        guid.encode(dst)?;

        dst.write_u8(self.id);

        match &self.property {
            CodecProperty::NsCodec(p) => {
                dst.write_u16(cast_length!("len", p.size())?);
                p.encode(dst)?;
            }
            CodecProperty::RemoteFx(p) => {
                match p {
                    RemoteFxContainer::ClientContainer(container) => {
                        dst.write_u16(cast_length!("len", container.size())?);
                        container.encode(dst)?;
                    }
                    RemoteFxContainer::ServerContainer(size) => {
                        dst.write_u16(cast_length!("len", *size)?);
                        let buff = vec![0u8; *size];
                        dst.write_slice(&buff);
                    }
                };
            }
            CodecProperty::ImageRemoteFx(p) => {
                match p {
                    RemoteFxContainer::ClientContainer(container) => {
                        dst.write_u16(cast_length!("len", container.size())?);
                        container.encode(dst)?;
                    }
                    RemoteFxContainer::ServerContainer(size) => {
                        dst.write_u16(cast_length!("len", *size)?);
                        let buff = vec![0u8; *size];
                        dst.write_slice(&buff);
                    }
                };
            }
            CodecProperty::Ignore => dst.write_u16(0),
            CodecProperty::None => dst.write_u16(0),
        };

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            + match &self.property {
                CodecProperty::NsCodec(p) => p.size(),
                CodecProperty::RemoteFx(p) => match p {
                    RemoteFxContainer::ClientContainer(container) => container.size(),
                    RemoteFxContainer::ServerContainer(size) => *size,
                },
                CodecProperty::ImageRemoteFx(p) => match p {
                    RemoteFxContainer::ClientContainer(container) => container.size(),
                    RemoteFxContainer::ServerContainer(size) => *size,
                },
                CodecProperty::Ignore => 0,
                CodecProperty::None => 0,
            }
    }
}

impl<'de> PduDecode<'de> for Codec {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let guid = Guid::decode(src)?;

        let id = src.read_u8();
        let codec_properties_len = usize::from(src.read_u16());

        let property = if codec_properties_len != 0 {
            ensure_size!(in: src, size: codec_properties_len);
            let property_buffer = src.read_slice(codec_properties_len);

            match guid {
                GUID_NSCODEC => CodecProperty::NsCodec(decode(property_buffer)?),
                GUID_REMOTEFX | GUID_IMAGE_REMOTEFX => {
                    let property = if property_buffer[0] == 0 {
                        RemoteFxContainer::ServerContainer(codec_properties_len)
                    } else {
                        RemoteFxContainer::ClientContainer(decode(property_buffer)?)
                    };

                    match guid {
                        GUID_REMOTEFX => CodecProperty::RemoteFx(property),
                        GUID_IMAGE_REMOTEFX => CodecProperty::ImageRemoteFx(property),
                        _ => unreachable!(),
                    }
                }
                GUID_IGNORE => CodecProperty::Ignore,
                _ => CodecProperty::None,
            }
        } else {
            match guid {
                GUID_NSCODEC | GUID_REMOTEFX | GUID_IMAGE_REMOTEFX => {
                    return Err(invalid_field_err!(
                        "codecPropertiesLen",
                        "invalid codec property length"
                    ));
                }
                GUID_IGNORE => CodecProperty::Ignore,
                _ => CodecProperty::None,
            }
        };

        Ok(Self { id, property })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RemoteFxContainer {
    ClientContainer(RfxClientCapsContainer),
    ServerContainer(usize),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum CodecProperty {
    NsCodec(NsCodec),
    RemoteFx(RemoteFxContainer),
    ImageRemoteFx(RemoteFxContainer),
    Ignore,
    None,
}

/// The NsCodec structure advertises properties of the NSCodec Bitmap Codec.
///
/// # Fields
///
/// * `is_dynamic_fidelity_allowed` - indicates support for lossy bitmap compression by reducing color fidelity
/// * `is_subsampling_allowed` - indicates support for chroma subsampling
/// * `color_loss_level` - indicates the maximum supported Color Loss Level
///
/// If received Color Loss Level value is lesser than 1 or greater than 7, it assigns to 1 or 7 respectively. This was made for compatibility with FreeRDP server.
///
/// # MSDN
///
/// * [NSCodec Capability Set](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpnsc/0eac0ba8-7bdd-4300-ab8d-9bc784c0a669)
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct NsCodec {
    pub is_dynamic_fidelity_allowed: bool,
    pub is_subsampling_allowed: bool,
    pub color_loss_level: u8,
}

impl NsCodec {
    const NAME: &'static str = "NsCodec";

    const FIXED_PART_SIZE: usize = NSCODEC_LENGTH;
}

impl PduEncode for NsCodec {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u8(u8::from(self.is_dynamic_fidelity_allowed));
        dst.write_u8(u8::from(self.is_subsampling_allowed));
        dst.write_u8(self.color_loss_level);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for NsCodec {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let is_dynamic_fidelity_allowed = src.read_u8() != 0;
        let is_subsampling_allowed = src.read_u8() != 0;

        let color_loss_level = src.read_u8().clamp(1, 7);

        Ok(Self {
            is_dynamic_fidelity_allowed,
            is_subsampling_allowed,
            color_loss_level,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxClientCapsContainer {
    pub capture_flags: CaptureFlags,
    pub caps_data: RfxCaps,
}

impl RfxClientCapsContainer {
    const NAME: &'static str = "RfxClientCapsContainer";

    const FIXED_PART_SIZE: usize = RFX_CLIENT_CAPS_CONTAINER_STATIC_DATA_LENGTH;
}

impl PduEncode for RfxClientCapsContainer {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(cast_length!("len", self.size())?);
        dst.write_u32(self.capture_flags.bits());
        dst.write_u32(cast_length!("capsLen", self.caps_data.size())?);
        self.caps_data.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.caps_data.size()
    }
}

impl<'de> PduDecode<'de> for RfxClientCapsContainer {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _length = src.read_u32();
        let capture_flags = CaptureFlags::from_bits_truncate(src.read_u32());
        let _caps_length = src.read_u32();
        let caps_data = RfxCaps::decode(src)?;

        Ok(Self {
            capture_flags,
            caps_data,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxCaps(pub RfxCapset);

impl RfxCaps {
    const NAME: &'static str = "RfxCaps";

    const FIXED_PART_SIZE: usize = RFX_CAPS_STATIC_DATA_LENGTH;
}

impl PduEncode for RfxCaps {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(RFX_CAPS_BLOCK_TYPE);
        dst.write_u32(RFX_CAPS_BLOCK_LENGTH);
        dst.write_u16(RFX_CAPS_NUM_CAPSETS);
        self.0.encode(dst)?; // capsets data

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.0.size()
    }
}

impl<'de> PduDecode<'de> for RfxCaps {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let block_type = src.read_u16();
        if block_type != RFX_CAPS_BLOCK_TYPE {
            return Err(invalid_field_err!("blockType", "invalid rfx caps block type"));
        }

        let block_len = src.read_u32();
        if block_len != RFX_CAPS_BLOCK_LENGTH {
            return Err(invalid_field_err!("blockLen", "invalid rfx caps block length"));
        }

        let num_capsets = src.read_u16();
        if num_capsets != RFX_CAPS_NUM_CAPSETS {
            return Err(invalid_field_err!("numCapsets", "invalid rfx caps num capsets"));
        }

        let capsets_data = RfxCapset::decode(src)?;

        Ok(RfxCaps(capsets_data))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxCapset(pub Vec<RfxICap>);

impl RfxCapset {
    const NAME: &'static str = "RfxCapset";

    const FIXED_PART_SIZE: usize = RFX_CAPSET_STATIC_DATA_LENGTH;
}

impl PduEncode for RfxCapset {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(RFX_CAPSET_BLOCK_TYPE);
        dst.write_u32(cast_length!(
            "len",
            RFX_CAPSET_STATIC_DATA_LENGTH + self.0.len() * RFX_ICAP_LENGTH
        )?);
        dst.write_u8(1); // codec id
        dst.write_u16(RFX_CAPSET_TYPE);
        dst.write_u16(cast_length!("len", self.0.len())?);
        dst.write_u16(cast_length!("len", RFX_ICAP_LENGTH)?);

        for rfx in self.0.iter() {
            rfx.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.0.len() * RFX_ICAP_LENGTH
    }
}

impl<'de> PduDecode<'de> for RfxCapset {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let block_type = src.read_u16();
        if block_type != RFX_CAPSET_BLOCK_TYPE {
            return Err(invalid_field_err!("blockType", "invalid rfx capset block type"));
        }

        let _block_len = src.read_u32();

        let codec_id = src.read_u8();
        if codec_id != 1 {
            return Err(invalid_field_err!("codecId", "invalid rfx codec ID"));
        }

        let capset_type = src.read_u16();
        if capset_type != RFX_CAPSET_TYPE {
            return Err(invalid_field_err!("capsetType", "invalid rfx capset type"));
        }

        let num_icaps = src.read_u16();
        let _icaps_len = src.read_u16();

        let mut icaps_data = Vec::with_capacity(num_icaps as usize);
        for _ in 0..num_icaps {
            icaps_data.push(RfxICap::decode(src)?);
        }

        Ok(RfxCapset(icaps_data))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxICap {
    pub flags: RfxICapFlags,
    pub entropy_bits: EntropyBits,
}

impl RfxICap {
    const NAME: &'static str = "RfxICap";

    const FIXED_PART_SIZE: usize = RFX_ICAP_LENGTH;
}

impl PduEncode for RfxICap {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(RFX_ICAP_VERSION);
        dst.write_u16(RFX_ICAP_TILE_SIZE);
        dst.write_u8(self.flags.bits());
        dst.write_u8(RFX_ICAP_COLOR_CONVERSION);
        dst.write_u8(RFX_ICAP_TRANSFORM_BITS);
        dst.write_u8(self.entropy_bits.to_u8().unwrap());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for RfxICap {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version = src.read_u16();
        if version != RFX_ICAP_VERSION {
            return Err(invalid_field_err!("version", "invalid rfx icap version"));
        }

        let tile_size = src.read_u16();
        if tile_size != RFX_ICAP_TILE_SIZE {
            return Err(invalid_field_err!("tileSize", "invalid rfx icap tile size"));
        }

        let flags = RfxICapFlags::from_bits_truncate(src.read_u8());

        let color_conversion = src.read_u8();
        if color_conversion != RFX_ICAP_COLOR_CONVERSION {
            return Err(invalid_field_err!("colorConv", "invalid rfx color conversion bits"));
        }

        let transform_bits = src.read_u8();
        if transform_bits != RFX_ICAP_TRANSFORM_BITS {
            return Err(invalid_field_err!("transformBits", "invalid rfx transform bits"));
        }

        let entropy_bits = EntropyBits::from_u8(src.read_u8())
            .ok_or_else(|| invalid_field_err!("entropyBits", "invalid rfx entropy bits"))?;

        Ok(RfxICap { flags, entropy_bits })
    }
}

#[derive(PartialEq, Eq, Debug, FromPrimitive, ToPrimitive, Copy, Clone)]
pub enum EntropyBits {
    Rlgr1 = 1,
    Rlgr3 = 4,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CaptureFlags: u32 {
        const CARDP_CAPS_CAPTURE_NON_CAC = 1;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct RfxICapFlags: u8 {
        const CODEC_MODE = 2;
    }
}
