#[cfg(test)]
pub mod test;

mod bitmap;
mod bitmap_cache;
mod bitmap_codecs;
mod brush;
mod general;
mod glyph_cache;
mod input;
mod offscreen_bitmap_cache;
mod order;
mod pointer;
mod sound;
mod surface_commands;
mod virtual_channel;

use std::io;

pub use self::{
    bitmap::{Bitmap, BitmapDrawingFlags},
    bitmap_cache::{
        BitmapCache, BitmapCacheRev2, CacheEntry, CacheFlags, CellInfo, BITMAP_CACHE_ENTRIES_NUM,
    },
    bitmap_codecs::{
        BitmapCodecs, CaptureFlags, Codec, CodecProperty, EntropyBits, Guid, NsCodec,
        RemoteFxContainer, RfxCaps, RfxCapset, RfxClientCapsContainer, RfxICap, RfxICapFlags,
    },
    brush::{Brush, SupportLevel},
    general::{General, GeneralExtraFlags, MajorPlatformType, MinorPlatformType},
    glyph_cache::{CacheDefinition, GlyphCache, GlyphSupportLevel, GLYPH_CACHE_NUM},
    input::{Input, InputFlags},
    offscreen_bitmap_cache::OffscreenBitmapCache,
    order::{Order, OrderFlags, OrderSupportExFlags, OrderSupportIndex},
    pointer::Pointer,
    sound::{Sound, SoundFlags},
    surface_commands::{CmdFlags, SurfaceCommands},
    virtual_channel::{VirtualChannel, VirtualChannelFlags},
};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{impl_from_error, PduParsing};

const SOURCE_DESCRIPTOR_LENGTH_FIELD_SIZE: usize = 2;
const COMBINED_CAPABILITIES_LENGTH_FIELD_SIZE: usize = 2;
const NUMBER_CAPABILITIES_FIELD_SIZE: usize = 2;
const PADDING_SIZE: usize = 2;
const SESSION_ID_FIELD_SIZE: usize = 4;
const CAPABILITY_SET_TYPE_FIELD_SIZE: usize = 2;
const CAPABILITY_SET_LENGTH_FIELD_SIZE: usize = 2;
const ORIGINATOR_ID_FIELD_SIZE: usize = 2;

const NULL_TERMINATOR: &str = "\0";
const SERVER_CHANNEL_ID: u16 = 0x03ea;

#[derive(Debug, Clone, PartialEq)]
pub struct ServerDemandActive {
    pub pdu: DemandActive,
}

impl PduParsing for ServerDemandActive {
    type Error = CapabilitySetsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let pdu = DemandActive::from_buffer(&mut stream)?;
        let _session_id = stream.read_u32::<LittleEndian>()?;

        Ok(Self { pdu })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.pdu.to_buffer(&mut stream)?;
        stream.write_u32::<LittleEndian>(0)?; // This field is ignored by the client

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.pdu.buffer_length() + SESSION_ID_FIELD_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientConfirmActive {
    pub pdu: DemandActive,
}

impl ClientConfirmActive {
    pub fn new(pdu: DemandActive) -> Self {
        Self { pdu }
    }
}

impl PduParsing for ClientConfirmActive {
    type Error = CapabilitySetsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let originator_id = stream.read_u16::<LittleEndian>()?;
        if originator_id == SERVER_CHANNEL_ID {
            Ok(Self {
                pdu: DemandActive::from_buffer(&mut stream)?,
            })
        } else {
            Err(CapabilitySetsError::InvalidOriginatorId)
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(SERVER_CHANNEL_ID)?;

        self.pdu.to_buffer(&mut stream)
    }

    fn buffer_length(&self) -> usize {
        self.pdu.buffer_length() + ORIGINATOR_ID_FIELD_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DemandActive {
    pub source_descriptor: String,
    pub capability_sets: Vec<CapabilitySet>,
}

impl DemandActive {
    pub fn new(source_descriptor: String, capability_sets: Vec<CapabilitySet>) -> Self {
        Self {
            source_descriptor,
            capability_sets,
        }
    }
}

impl PduParsing for DemandActive {
    type Error = CapabilitySetsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let source_descriptor_length = stream.read_u16::<LittleEndian>()? as usize;
        // The combined size in bytes of the numberCapabilities, pad2Octets, and capabilitySets fields.
        let _combined_capabilities_length = stream.read_u16::<LittleEndian>()? as usize;

        let mut source_descriptor_buffer = vec![0; source_descriptor_length];
        stream.read_exact(source_descriptor_buffer.as_mut())?;
        let source_descriptor = String::from_utf8(source_descriptor_buffer)?
            .trim_end_matches(NULL_TERMINATOR)
            .to_string();

        let capability_sets_count = stream.read_u16::<LittleEndian>()? as usize;
        let _padding = stream.read_u16::<LittleEndian>()?;

        let mut capability_sets = Vec::with_capacity(capability_sets_count);
        for _ in 0..capability_sets_count {
            capability_sets.push(CapabilitySet::from_buffer(&mut stream)?);
        }

        Ok(Self {
            source_descriptor,
            capability_sets,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let combined_length = self
            .capability_sets
            .iter()
            .map(PduParsing::buffer_length)
            .sum::<usize>()
            + NUMBER_CAPABILITIES_FIELD_SIZE
            + PADDING_SIZE;

        stream.write_u16::<LittleEndian>(
            (self.source_descriptor.len() + NULL_TERMINATOR.as_bytes().len()) as u16,
        )?;
        stream.write_u16::<LittleEndian>(combined_length as u16)?;
        stream.write_all(self.source_descriptor.as_ref())?;
        stream.write_all(NULL_TERMINATOR.as_bytes())?;
        stream.write_u16::<LittleEndian>(self.capability_sets.len() as u16)?;
        stream.write_u16::<LittleEndian>(0)?; // padding

        for capability_set in self.capability_sets.iter() {
            capability_set.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SOURCE_DESCRIPTOR_LENGTH_FIELD_SIZE
            + COMBINED_CAPABILITIES_LENGTH_FIELD_SIZE
            + self.source_descriptor.len()
            + 1
            + NUMBER_CAPABILITIES_FIELD_SIZE
            + PADDING_SIZE
            + self
                .capability_sets
                .iter()
                .map(PduParsing::buffer_length)
                .sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CapabilitySet {
    // mandatory
    General(General),
    Bitmap(Bitmap),
    Order(Order),
    BitmapCache(BitmapCache),
    BitmapCacheRev2(BitmapCacheRev2),
    Pointer(Pointer),
    Sound(Sound),
    Input(Input),
    Brush(Brush),
    GlyphCache(GlyphCache),
    OffscreenBitmapCache(OffscreenBitmapCache),
    VirtualChannel(VirtualChannel),

    // optional
    Control(Vec<u8>),
    WindowActivation(Vec<u8>),
    Share(Vec<u8>),
    Font(Vec<u8>),
    BitmapCacheHostSupport(Vec<u8>),
    DesktopComposition(Vec<u8>),
    MultiFragmentUpdate(Vec<u8>),
    LargePointer(Vec<u8>),
    SurfaceCommands(SurfaceCommands),
    BitmapCodecs(BitmapCodecs),

    // other
    ColorCache(Vec<u8>),
    DrawNineGridCache(Vec<u8>),
    DrawGdiPlus(Vec<u8>),
    Rail(Vec<u8>),
    WindowList(Vec<u8>),
    FrameAcknowledge(Vec<u8>),
}

impl PduParsing for CapabilitySet {
    type Error = CapabilitySetsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let capability_set_type = CapabilitySetType::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(CapabilitySetsError::InvalidType)?;
        let buffer_length = stream.read_u16::<LittleEndian>()? as usize
            - CAPABILITY_SET_TYPE_FIELD_SIZE
            - CAPABILITY_SET_LENGTH_FIELD_SIZE;

        match capability_set_type {
            CapabilitySetType::General => {
                Ok(CapabilitySet::General(General::from_buffer(&mut stream)?))
            }
            CapabilitySetType::Bitmap => {
                Ok(CapabilitySet::Bitmap(Bitmap::from_buffer(&mut stream)?))
            }
            CapabilitySetType::Order => Ok(CapabilitySet::Order(Order::from_buffer(&mut stream)?)),
            CapabilitySetType::BitmapCache => Ok(CapabilitySet::BitmapCache(
                BitmapCache::from_buffer(&mut stream)?,
            )),
            CapabilitySetType::BitmapCacheRev2 => Ok(CapabilitySet::BitmapCacheRev2(
                BitmapCacheRev2::from_buffer(&mut stream)?,
            )),
            CapabilitySetType::Pointer => {
                Ok(CapabilitySet::Pointer(Pointer::from_buffer(&mut stream)?))
            }
            CapabilitySetType::Sound => Ok(CapabilitySet::Sound(Sound::from_buffer(&mut stream)?)),
            CapabilitySetType::Input => Ok(CapabilitySet::Input(Input::from_buffer(&mut stream)?)),
            CapabilitySetType::Brush => Ok(CapabilitySet::Brush(Brush::from_buffer(&mut stream)?)),
            CapabilitySetType::GlyphCache => Ok(CapabilitySet::GlyphCache(
                GlyphCache::from_buffer(&mut stream)?,
            )),
            CapabilitySetType::OffscreenBitmapCache => Ok(CapabilitySet::OffscreenBitmapCache(
                OffscreenBitmapCache::from_buffer(&mut stream)?,
            )),
            CapabilitySetType::VirtualChannel => Ok(CapabilitySet::VirtualChannel(
                VirtualChannel::from_buffer(&mut stream)?,
            )),
            CapabilitySetType::SurfaceCommands => Ok(CapabilitySet::SurfaceCommands(
                SurfaceCommands::from_buffer(&mut stream)?,
            )),
            CapabilitySetType::BitmapCodecs => Ok(CapabilitySet::BitmapCodecs(
                BitmapCodecs::from_buffer(&mut stream)?,
            )),
            _ => {
                let mut capability_set_buffer = vec![0; buffer_length];
                stream.read_exact(capability_set_buffer.as_mut())?;

                match capability_set_type {
                    CapabilitySetType::Control => Ok(CapabilitySet::Control(capability_set_buffer)),
                    CapabilitySetType::WindowActivation => {
                        Ok(CapabilitySet::WindowActivation(capability_set_buffer))
                    }
                    CapabilitySetType::Share => Ok(CapabilitySet::Share(capability_set_buffer)),
                    CapabilitySetType::Font => Ok(CapabilitySet::Font(capability_set_buffer)),
                    CapabilitySetType::BitmapCacheHostSupport => {
                        Ok(CapabilitySet::BitmapCacheHostSupport(capability_set_buffer))
                    }
                    CapabilitySetType::DesktopComposition => {
                        Ok(CapabilitySet::DesktopComposition(capability_set_buffer))
                    }
                    CapabilitySetType::MultiFragmentUpdate => {
                        Ok(CapabilitySet::MultiFragmentUpdate(capability_set_buffer))
                    }
                    CapabilitySetType::LargePointer => {
                        Ok(CapabilitySet::LargePointer(capability_set_buffer))
                    }
                    CapabilitySetType::ColorCache => {
                        Ok(CapabilitySet::ColorCache(capability_set_buffer))
                    }
                    CapabilitySetType::DrawNineGridCache => {
                        Ok(CapabilitySet::DrawNineGridCache(capability_set_buffer))
                    }
                    CapabilitySetType::DrawGdiPlus => {
                        Ok(CapabilitySet::DrawGdiPlus(capability_set_buffer))
                    }
                    CapabilitySetType::Rail => Ok(CapabilitySet::Rail(capability_set_buffer)),
                    CapabilitySetType::WindowList => {
                        Ok(CapabilitySet::WindowList(capability_set_buffer))
                    }
                    CapabilitySetType::FrameAcknowledge => {
                        Ok(CapabilitySet::FrameAcknowledge(capability_set_buffer))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        match self {
            CapabilitySet::General(capset) => {
                stream.write_u16::<LittleEndian>(CapabilitySetType::General.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::Bitmap(capset) => {
                stream.write_u16::<LittleEndian>(CapabilitySetType::Bitmap.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::Order(capset) => {
                stream.write_u16::<LittleEndian>(CapabilitySetType::Order.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::BitmapCache(capset) => {
                stream
                    .write_u16::<LittleEndian>(CapabilitySetType::BitmapCache.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::BitmapCacheRev2(capset) => {
                stream.write_u16::<LittleEndian>(
                    CapabilitySetType::BitmapCacheRev2.to_u16().unwrap(),
                )?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::Pointer(capset) => {
                stream.write_u16::<LittleEndian>(CapabilitySetType::Pointer.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::Sound(capset) => {
                stream.write_u16::<LittleEndian>(CapabilitySetType::Sound.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::Input(capset) => {
                stream.write_u16::<LittleEndian>(CapabilitySetType::Input.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::Brush(capset) => {
                stream.write_u16::<LittleEndian>(CapabilitySetType::Brush.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::GlyphCache(capset) => {
                stream
                    .write_u16::<LittleEndian>(CapabilitySetType::GlyphCache.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::OffscreenBitmapCache(capset) => {
                stream.write_u16::<LittleEndian>(
                    CapabilitySetType::OffscreenBitmapCache.to_u16().unwrap(),
                )?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::VirtualChannel(capset) => {
                stream.write_u16::<LittleEndian>(
                    CapabilitySetType::VirtualChannel.to_u16().unwrap(),
                )?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::SurfaceCommands(capset) => {
                stream.write_u16::<LittleEndian>(
                    CapabilitySetType::SurfaceCommands.to_u16().unwrap(),
                )?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            CapabilitySet::BitmapCodecs(capset) => {
                stream
                    .write_u16::<LittleEndian>(CapabilitySetType::BitmapCodecs.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capset.buffer_length()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                capset.to_buffer(&mut stream)?;
            }
            _ => {
                let (capability_set_type, capability_set_buffer) = match self {
                    CapabilitySet::Control(buffer) => (CapabilitySetType::Control, buffer),
                    CapabilitySet::WindowActivation(buffer) => {
                        (CapabilitySetType::WindowActivation, buffer)
                    }
                    CapabilitySet::Share(buffer) => (CapabilitySetType::Share, buffer),
                    CapabilitySet::Font(buffer) => (CapabilitySetType::Font, buffer),
                    CapabilitySet::BitmapCacheHostSupport(buffer) => {
                        (CapabilitySetType::BitmapCacheHostSupport, buffer)
                    }
                    CapabilitySet::DesktopComposition(buffer) => {
                        (CapabilitySetType::DesktopComposition, buffer)
                    }
                    CapabilitySet::MultiFragmentUpdate(buffer) => {
                        (CapabilitySetType::MultiFragmentUpdate, buffer)
                    }
                    CapabilitySet::LargePointer(buffer) => {
                        (CapabilitySetType::LargePointer, buffer)
                    }
                    CapabilitySet::ColorCache(buffer) => (CapabilitySetType::ColorCache, buffer),
                    CapabilitySet::DrawNineGridCache(buffer) => {
                        (CapabilitySetType::DrawNineGridCache, buffer)
                    }
                    CapabilitySet::DrawGdiPlus(buffer) => (CapabilitySetType::DrawGdiPlus, buffer),
                    CapabilitySet::Rail(buffer) => (CapabilitySetType::Rail, buffer),
                    CapabilitySet::WindowList(buffer) => (CapabilitySetType::WindowList, buffer),
                    CapabilitySet::FrameAcknowledge(buffer) => {
                        (CapabilitySetType::FrameAcknowledge, buffer)
                    }
                    _ => unreachable!(),
                };

                stream.write_u16::<LittleEndian>(capability_set_type.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(
                    (capability_set_buffer.len()
                        + CAPABILITY_SET_TYPE_FIELD_SIZE
                        + CAPABILITY_SET_LENGTH_FIELD_SIZE) as u16,
                )?;
                stream.write_all(capability_set_buffer)?;
            }
        };
        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CAPABILITY_SET_TYPE_FIELD_SIZE
            + CAPABILITY_SET_LENGTH_FIELD_SIZE
            + match self {
                CapabilitySet::General(capset) => capset.buffer_length(),
                CapabilitySet::Bitmap(capset) => capset.buffer_length(),
                CapabilitySet::Order(capset) => capset.buffer_length(),
                CapabilitySet::BitmapCache(capset) => capset.buffer_length(),
                CapabilitySet::BitmapCacheRev2(capset) => capset.buffer_length(),
                CapabilitySet::Pointer(capset) => capset.buffer_length(),
                CapabilitySet::Sound(capset) => capset.buffer_length(),
                CapabilitySet::Input(capset) => capset.buffer_length(),
                CapabilitySet::Brush(capset) => capset.buffer_length(),
                CapabilitySet::GlyphCache(capset) => capset.buffer_length(),
                CapabilitySet::OffscreenBitmapCache(capset) => capset.buffer_length(),
                CapabilitySet::VirtualChannel(capset) => capset.buffer_length(),
                CapabilitySet::SurfaceCommands(capset) => capset.buffer_length(),
                CapabilitySet::BitmapCodecs(capset) => capset.buffer_length(),
                CapabilitySet::Control(buffer)
                | CapabilitySet::WindowActivation(buffer)
                | CapabilitySet::Share(buffer)
                | CapabilitySet::Font(buffer)
                | CapabilitySet::BitmapCacheHostSupport(buffer)
                | CapabilitySet::DesktopComposition(buffer)
                | CapabilitySet::MultiFragmentUpdate(buffer)
                | CapabilitySet::LargePointer(buffer)
                | CapabilitySet::ColorCache(buffer)
                | CapabilitySet::DrawNineGridCache(buffer)
                | CapabilitySet::DrawGdiPlus(buffer)
                | CapabilitySet::Rail(buffer)
                | CapabilitySet::WindowList(buffer)
                | CapabilitySet::FrameAcknowledge(buffer) => buffer.len(),
            }
    }
}

#[derive(Copy, Clone, Debug, FromPrimitive, ToPrimitive)]
enum CapabilitySetType {
    General = 0x01,
    Bitmap = 0x02,
    Order = 0x03,
    BitmapCache = 0x04,
    Control = 0x05,
    WindowActivation = 0x07,
    Pointer = 0x08,
    Share = 0x09,
    ColorCache = 0x0a,
    Sound = 0x0c,
    Input = 0x0d,
    Font = 0x0e,
    Brush = 0x0f,
    GlyphCache = 0x10,
    OffscreenBitmapCache = 0x11,
    BitmapCacheHostSupport = 0x12,
    BitmapCacheRev2 = 0x13,
    VirtualChannel = 0x14,
    DrawNineGridCache = 0x15,
    DrawGdiPlus = 0x16,
    Rail = 0x17,
    WindowList = 0x18,
    DesktopComposition = 0x19,
    MultiFragmentUpdate = 0x1a,
    LargePointer = 0x1b,
    SurfaceCommands = 0x1c,
    BitmapCodecs = 0x1d,
    FrameAcknowledge = 0x1e,
}

#[derive(Debug, Fail)]
pub enum CapabilitySetsError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Utf8 error: {}", _0)]
    Utf8Error(#[fail(cause)] std::string::FromUtf8Error),
    #[fail(display = "Invalid type field")]
    InvalidType,
    #[fail(display = "Invalid originator ID field")]
    InvalidOriginatorId,
    #[fail(display = "Invalid bitmap compression field")]
    InvalidCompressionFlag,
    #[fail(display = "Invalid multiple rectangle support field")]
    InvalidMultipleRectSupport,
    #[fail(display = "Invalid major platform type field")]
    InvalidMajorPlatformType,
    #[fail(display = "Invalid minor platform type field")]
    InvalidMinorPlatformType,
    #[fail(display = "Invalid protocol version field")]
    InvalidProtocolVersion,
    #[fail(display = "Invalid compression types field")]
    InvalidCompressionTypes,
    #[fail(display = "Invalid update capability flags field")]
    InvalidUpdateCapFlag,
    #[fail(display = "Invalid remote unshare flag field")]
    InvalidRemoteUnshareFlag,
    #[fail(display = "Invalid compression level field")]
    InvalidCompressionLevel,
    #[fail(display = "Invalid brush support level field")]
    InvalidBrushSupportLevel,
    #[fail(display = "Invalid glyph support level field")]
    InvalidGlyphSupportLevel,
    #[fail(display = "Invalid RemoteFX capability version")]
    InvalidRfxICapVersion,
    #[fail(display = "Invalid RemoteFX capability tile size")]
    InvalidRfxICapTileSize,
    #[fail(display = "Invalid RemoteFXICap color conversion bits")]
    InvalidRfxICapColorConvBits,
    #[fail(display = "Invalid RemoteFXICap transform bits")]
    InvalidRfxICapTransformBits,
    #[fail(display = "Invalid RemoteFXICap entropy bits field")]
    InvalidRfxICapEntropyBits,
    #[fail(display = "Invalid RemoteFX capability set block type")]
    InvalidRfxCapsetBlockType,
    #[fail(display = "Invalid RemoteFX capability set type")]
    InvalidRfxCapsetType,
    #[fail(display = "Invalid RemoteFX capabilities block type")]
    InvalidRfxCapsBlockType,
    #[fail(display = "Invalid RemoteFX capabilities block length")]
    InvalidRfxCapsBockLength,
    #[fail(display = "Invalid number of capability sets in RemoteFX capabilities")]
    InvalidRfxCapsNumCapsets,
    #[fail(display = "Invalid color loss level field")]
    InvalidColorLossLevel,
    #[fail(display = "Invalid codec property field")]
    InvalidCodecProperty,
    #[fail(display = "Invalid codec ID")]
    InvalidCodecID,
    #[fail(display = "Invalid channel chunk size field")]
    InvalidChunkSize,
    #[fail(display = "Invalid codec property length for the current property ID")]
    InvalidPropertyLength,
}

impl_from_error!(io::Error, CapabilitySetsError, CapabilitySetsError::IOError);

impl From<std::string::FromUtf8Error> for CapabilitySetsError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        CapabilitySetsError::Utf8Error(e)
    }
}
