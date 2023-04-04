#[cfg(test)]
mod tests;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::rdp::capability_sets::CapabilitySetsError;
use crate::PduParsing;

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
const BITMAP_CODECS_STATIC_DATA: usize = 1;

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

impl PduParsing for Guid {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let guid1 = buffer.read_u32::<LittleEndian>()?;
        let guid2 = buffer.read_u16::<LittleEndian>()?;
        let guid3 = buffer.read_u16::<LittleEndian>()?;
        let guid4 = buffer.read_u8()?;
        let guid5 = buffer.read_u8()?;
        let guid6 = buffer.read_u8()?;
        let guid7 = buffer.read_u8()?;
        let guid8 = buffer.read_u8()?;
        let guid9 = buffer.read_u8()?;
        let guid10 = buffer.read_u8()?;
        let guid11 = buffer.read_u8()?;

        Ok(Guid(
            guid1, guid2, guid3, guid4, guid5, guid6, guid7, guid8, guid9, guid10, guid11,
        ))
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.0)?;
        buffer.write_u16::<LittleEndian>(self.1)?;
        buffer.write_u16::<LittleEndian>(self.2)?;
        buffer.write_u8(self.3)?;
        buffer.write_u8(self.4)?;
        buffer.write_u8(self.5)?;
        buffer.write_u8(self.6)?;
        buffer.write_u8(self.7)?;
        buffer.write_u8(self.8)?;
        buffer.write_u8(self.9)?;
        buffer.write_u8(self.10)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        16_usize
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BitmapCodecs(pub Vec<Codec>);

impl PduParsing for BitmapCodecs {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let codecs_count = buffer.read_u8()?;

        let mut codecs = Vec::with_capacity(codecs_count as usize);
        for _ in 0..codecs_count {
            codecs.push(Codec::from_buffer(&mut buffer)?);
        }

        Ok(BitmapCodecs(codecs))
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u8(self.0.len() as u8)?;

        for codec in self.0.iter() {
            codec.to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.0.iter().map(PduParsing::buffer_length).sum::<usize>() + BITMAP_CODECS_STATIC_DATA
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Codec {
    pub id: u8,
    pub property: CodecProperty,
}

impl PduParsing for Codec {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let guid = Guid::from_buffer(&mut buffer)?;

        let id = buffer.read_u8()?;
        let codec_properties_len = usize::from(buffer.read_u16::<LittleEndian>()?);

        let property = if codec_properties_len != 0 {
            let mut property_buffer = vec![0u8; codec_properties_len];
            buffer.read_exact(&mut property_buffer)?;

            match guid {
                GUID_NSCODEC => CodecProperty::NsCodec(NsCodec::from_buffer(&mut property_buffer.as_slice())?),
                GUID_REMOTEFX | GUID_IMAGE_REMOTEFX => {
                    let property = if property_buffer[0] == 0 {
                        RemoteFxContainer::ServerContainer(codec_properties_len)
                    } else {
                        RemoteFxContainer::ClientContainer(RfxClientCapsContainer::from_buffer(
                            &mut property_buffer.as_slice(),
                        )?)
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
                    return Err(CapabilitySetsError::InvalidPropertyLength)
                }
                GUID_IGNORE => CodecProperty::Ignore,
                _ => CodecProperty::None,
            }
        };

        Ok(Self { id, property })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        let guid = match &self.property {
            CodecProperty::NsCodec(_) => GUID_NSCODEC,
            CodecProperty::RemoteFx(_) => GUID_REMOTEFX,
            CodecProperty::ImageRemoteFx(_) => GUID_IMAGE_REMOTEFX,
            CodecProperty::Ignore => GUID_IGNORE,
            _ => return Err(CapabilitySetsError::InvalidCodecID),
        };
        guid.to_buffer(&mut buffer)?;

        buffer.write_u8(self.id)?;

        match &self.property {
            CodecProperty::NsCodec(p) => {
                buffer.write_u16::<LittleEndian>(p.buffer_length() as u16)?;
                p.to_buffer(&mut buffer)?;
            }
            CodecProperty::RemoteFx(p) => {
                match p {
                    RemoteFxContainer::ClientContainer(container) => {
                        buffer.write_u16::<LittleEndian>(container.buffer_length() as u16)?;
                        container.to_buffer(&mut buffer)?;
                    }
                    RemoteFxContainer::ServerContainer(size) => {
                        buffer.write_u16::<LittleEndian>(*size as u16)?;
                        let buff = vec![0u8; *size];
                        buffer.write_all(&buff)?;
                    }
                };
            }
            CodecProperty::ImageRemoteFx(p) => {
                match p {
                    RemoteFxContainer::ClientContainer(container) => {
                        buffer.write_u16::<LittleEndian>(container.buffer_length() as u16)?;
                        container.to_buffer(&mut buffer)?;
                    }
                    RemoteFxContainer::ServerContainer(size) => {
                        buffer.write_u16::<LittleEndian>(*size as u16)?;
                        let buff = vec![0u8; *size];
                        buffer.write_all(&buff)?;
                    }
                };
            }
            CodecProperty::Ignore => buffer.write_u16::<LittleEndian>(0)?,
            CodecProperty::None => buffer.write_u16::<LittleEndian>(0)?,
        };

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CODEC_STATIC_DATA_LENGTH
            + match &self.property {
                CodecProperty::NsCodec(p) => p.buffer_length(),
                CodecProperty::RemoteFx(p) => match p {
                    RemoteFxContainer::ClientContainer(container) => container.buffer_length(),
                    RemoteFxContainer::ServerContainer(size) => *size,
                },
                CodecProperty::ImageRemoteFx(p) => match p {
                    RemoteFxContainer::ClientContainer(container) => container.buffer_length(),
                    RemoteFxContainer::ServerContainer(size) => *size,
                },
                CodecProperty::Ignore => 0,
                CodecProperty::None => 0,
            }
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

impl PduParsing for NsCodec {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let is_dynamic_fidelity_allowed = buffer.read_u8()? != 0;
        let is_subsampling_allowed = buffer.read_u8()? != 0;

        let color_loss_level = buffer.read_u8()?.max(1).min(7);

        Ok(Self {
            is_dynamic_fidelity_allowed,
            is_subsampling_allowed,
            color_loss_level,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u8(u8::from(self.is_dynamic_fidelity_allowed))?;
        buffer.write_u8(u8::from(self.is_subsampling_allowed))?;
        buffer.write_u8(self.color_loss_level)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        NSCODEC_LENGTH
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxClientCapsContainer {
    pub capture_flags: CaptureFlags,
    pub caps_data: RfxCaps,
}

impl PduParsing for RfxClientCapsContainer {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let _length = buffer.read_u32::<LittleEndian>()?;
        let capture_flags = CaptureFlags::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let _caps_length = buffer.read_u32::<LittleEndian>()?;
        let caps_data = RfxCaps::from_buffer(&mut buffer)?;

        Ok(Self {
            capture_flags,
            caps_data,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.buffer_length() as u32)?;
        buffer.write_u32::<LittleEndian>(self.capture_flags.bits())?;
        buffer.write_u32::<LittleEndian>(self.caps_data.buffer_length() as u32)?;
        self.caps_data.to_buffer(&mut buffer)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RFX_CLIENT_CAPS_CONTAINER_STATIC_DATA_LENGTH + self.caps_data.buffer_length()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxCaps(pub RfxCapset);

impl PduParsing for RfxCaps {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let block_type = buffer.read_u16::<LittleEndian>()?;
        if block_type != RFX_CAPS_BLOCK_TYPE {
            return Err(CapabilitySetsError::InvalidRfxCapsBlockType);
        }

        let block_len = buffer.read_u32::<LittleEndian>()?;
        if block_len != RFX_CAPS_BLOCK_LENGTH {
            return Err(CapabilitySetsError::InvalidRfxCapsBockLength);
        }

        let num_capsets = buffer.read_u16::<LittleEndian>()?;
        if num_capsets != RFX_CAPS_NUM_CAPSETS {
            return Err(CapabilitySetsError::InvalidRfxCapsNumCapsets);
        }

        let capsets_data = RfxCapset::from_buffer(&mut buffer)?;

        Ok(RfxCaps(capsets_data))
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(RFX_CAPS_BLOCK_TYPE)?;
        buffer.write_u32::<LittleEndian>(RFX_CAPS_BLOCK_LENGTH)?;
        buffer.write_u16::<LittleEndian>(RFX_CAPS_NUM_CAPSETS)?;
        self.0.to_buffer(&mut buffer)?; // capsets data

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RFX_CAPS_STATIC_DATA_LENGTH + self.0.buffer_length()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxCapset(pub Vec<RfxICap>);

impl PduParsing for RfxCapset {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let block_type = buffer.read_u16::<LittleEndian>()?;
        if block_type != RFX_CAPSET_BLOCK_TYPE {
            return Err(CapabilitySetsError::InvalidRfxCapsetBlockType);
        }

        let _block_len = buffer.read_u32::<LittleEndian>()?;

        let codec_id = buffer.read_u8()?;
        if codec_id != 1 {
            return Err(CapabilitySetsError::InvalidCodecID);
        }

        let capset_type = buffer.read_u16::<LittleEndian>()?;
        if capset_type != RFX_CAPSET_TYPE {
            return Err(CapabilitySetsError::InvalidRfxCapsetType);
        }

        let num_icaps = buffer.read_u16::<LittleEndian>()?;
        let _icaps_len = buffer.read_u16::<LittleEndian>()?;

        let mut icaps_data = Vec::with_capacity(num_icaps as usize);
        for _ in 0..num_icaps {
            icaps_data.push(RfxICap::from_buffer(&mut buffer)?);
        }

        Ok(RfxCapset(icaps_data))
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(RFX_CAPSET_BLOCK_TYPE)?;
        buffer.write_u32::<LittleEndian>((RFX_CAPSET_STATIC_DATA_LENGTH + self.0.len() * RFX_ICAP_LENGTH) as u32)?;
        buffer.write_u8(1)?; // codec id
        buffer.write_u16::<LittleEndian>(RFX_CAPSET_TYPE)?;
        buffer.write_u16::<LittleEndian>(self.0.len() as u16)?;
        buffer.write_u16::<LittleEndian>(RFX_ICAP_LENGTH as u16)?;

        for rfx in self.0.iter() {
            rfx.to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RFX_CAPSET_STATIC_DATA_LENGTH + self.0.len() * RFX_ICAP_LENGTH
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RfxICap {
    pub flags: RfxICapFlags,
    pub entropy_bits: EntropyBits,
}

impl PduParsing for RfxICap {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let version = buffer.read_u16::<LittleEndian>()?;
        if version != RFX_ICAP_VERSION {
            return Err(CapabilitySetsError::InvalidRfxICapVersion);
        }

        let tile_size = buffer.read_u16::<LittleEndian>()?;
        if tile_size != RFX_ICAP_TILE_SIZE {
            return Err(CapabilitySetsError::InvalidRfxICapTileSize);
        }

        let flags = RfxICapFlags::from_bits_truncate(buffer.read_u8()?);

        let color_conversion = buffer.read_u8()?;
        if color_conversion != RFX_ICAP_COLOR_CONVERSION {
            return Err(CapabilitySetsError::InvalidRfxICapColorConvBits);
        }

        let transform_bits = buffer.read_u8()?;
        if transform_bits != RFX_ICAP_TRANSFORM_BITS {
            return Err(CapabilitySetsError::InvalidRfxICapTransformBits);
        }

        let entropy_bits =
            EntropyBits::from_u8(buffer.read_u8()?).ok_or(CapabilitySetsError::InvalidRfxICapEntropyBits)?;

        Ok(RfxICap { flags, entropy_bits })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(RFX_ICAP_VERSION)?;
        buffer.write_u16::<LittleEndian>(RFX_ICAP_TILE_SIZE)?;
        buffer.write_u8(self.flags.bits())?;
        buffer.write_u8(RFX_ICAP_COLOR_CONVERSION)?;
        buffer.write_u8(RFX_ICAP_TRANSFORM_BITS)?;
        buffer.write_u8(self.entropy_bits.to_u8().unwrap())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RFX_ICAP_LENGTH
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
