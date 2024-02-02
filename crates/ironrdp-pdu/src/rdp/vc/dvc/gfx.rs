mod graphics_messages;

pub use graphics_messages::{
    Avc420BitmapStream, Avc444BitmapStream, CacheImportReplyPdu, CacheToSurfacePdu, CapabilitiesAdvertisePdu,
    CapabilitiesConfirmPdu, CapabilitiesV103Flags, CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitiesV10Flags,
    CapabilitiesV81Flags, CapabilitiesV8Flags, CapabilitySet, Codec1Type, Codec2Type, Color, CreateSurfacePdu,
    DeleteEncodingContextPdu, DeleteSurfacePdu, Encoding, EndFramePdu, EvictCacheEntryPdu, FrameAcknowledgePdu,
    MapSurfaceToOutputPdu, MapSurfaceToScaledOutputPdu, MapSurfaceToScaledWindowPdu, PixelFormat, Point, QuantQuality,
    QueueDepth, ResetGraphicsPdu, SolidFillPdu, StartFramePdu, SurfaceToCachePdu, SurfaceToSurfacePdu, Timestamp,
    WireToSurface1Pdu, WireToSurface2Pdu,
};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPdu {
    WireToSurface1(WireToSurface1Pdu),
    WireToSurface2(WireToSurface2Pdu),
    DeleteEncodingContext(DeleteEncodingContextPdu),
    SolidFill(SolidFillPdu),
    SurfaceToSurface(SurfaceToSurfacePdu),
    SurfaceToCache(SurfaceToCachePdu),
    CacheToSurface(CacheToSurfacePdu),
    EvictCacheEntry(EvictCacheEntryPdu),
    CreateSurface(CreateSurfacePdu),
    DeleteSurface(DeleteSurfacePdu),
    StartFrame(StartFramePdu),
    EndFrame(EndFramePdu),
    ResetGraphics(ResetGraphicsPdu),
    MapSurfaceToOutput(MapSurfaceToOutputPdu),
    CapabilitiesConfirm(CapabilitiesConfirmPdu),
    CacheImportReply(CacheImportReplyPdu),
    MapSurfaceToScaledOutput(MapSurfaceToScaledOutputPdu),
    MapSurfaceToScaledWindow(MapSurfaceToScaledWindowPdu),
}

const RDP_GFX_HEADER_SIZE: usize = 2 /* PduType */ + 2 /* flags */ + 4 /* bufferLen */;

impl ServerPdu {
    const NAME: &'static str = "GfxServerPdu";

    const FIXED_PART_SIZE: usize = RDP_GFX_HEADER_SIZE;
}

impl PduEncode for ServerPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let buffer_length = self.size();

        dst.write_u16(ServerPduType::from(self).to_u16().unwrap());
        dst.write_u16(0); // flags
        dst.write_u32(cast_length!("bufferLen", buffer_length)?);

        match self {
            ServerPdu::WireToSurface1(pdu) => pdu.encode(dst),
            ServerPdu::WireToSurface2(pdu) => pdu.encode(dst),
            ServerPdu::DeleteEncodingContext(pdu) => pdu.encode(dst),
            ServerPdu::SolidFill(pdu) => pdu.encode(dst),
            ServerPdu::SurfaceToSurface(pdu) => pdu.encode(dst),
            ServerPdu::SurfaceToCache(pdu) => pdu.encode(dst),
            ServerPdu::CacheToSurface(pdu) => pdu.encode(dst),
            ServerPdu::CreateSurface(pdu) => pdu.encode(dst),
            ServerPdu::DeleteSurface(pdu) => pdu.encode(dst),
            ServerPdu::ResetGraphics(pdu) => pdu.encode(dst),
            ServerPdu::MapSurfaceToOutput(pdu) => pdu.encode(dst),
            ServerPdu::MapSurfaceToScaledOutput(pdu) => pdu.encode(dst),
            ServerPdu::MapSurfaceToScaledWindow(pdu) => pdu.encode(dst),
            ServerPdu::StartFrame(pdu) => pdu.encode(dst),
            ServerPdu::EndFrame(pdu) => pdu.encode(dst),
            ServerPdu::EvictCacheEntry(pdu) => pdu.encode(dst),
            ServerPdu::CapabilitiesConfirm(pdu) => pdu.encode(dst),
            ServerPdu::CacheImportReply(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            + match self {
                ServerPdu::WireToSurface1(pdu) => pdu.size(),
                ServerPdu::WireToSurface2(pdu) => pdu.size(),
                ServerPdu::DeleteEncodingContext(pdu) => pdu.size(),
                ServerPdu::SolidFill(pdu) => pdu.size(),
                ServerPdu::SurfaceToSurface(pdu) => pdu.size(),
                ServerPdu::SurfaceToCache(pdu) => pdu.size(),
                ServerPdu::CacheToSurface(pdu) => pdu.size(),
                ServerPdu::CreateSurface(pdu) => pdu.size(),
                ServerPdu::DeleteSurface(pdu) => pdu.size(),
                ServerPdu::ResetGraphics(pdu) => pdu.size(),
                ServerPdu::MapSurfaceToOutput(pdu) => pdu.size(),
                ServerPdu::MapSurfaceToScaledOutput(pdu) => pdu.size(),
                ServerPdu::MapSurfaceToScaledWindow(pdu) => pdu.size(),
                ServerPdu::StartFrame(pdu) => pdu.size(),
                ServerPdu::EndFrame(pdu) => pdu.size(),
                ServerPdu::EvictCacheEntry(pdu) => pdu.size(),
                ServerPdu::CapabilitiesConfirm(pdu) => pdu.size(),
                ServerPdu::CacheImportReply(pdu) => pdu.size(),
            }
    }
}

impl<'a> PduDecode<'a> for ServerPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pdu_type = ServerPduType::from_u16(src.read_u16())
            .ok_or_else(|| invalid_message_err!("serverPduType", "invalid pdu type"))?;
        let _flags = src.read_u16();
        let pdu_length = cast_length!("pduLen", src.read_u32())?;

        let (server_pdu, buffer_length) = {
            let pdu = match pdu_type {
                ServerPduType::DeleteEncodingContext => {
                    ServerPdu::DeleteEncodingContext(DeleteEncodingContextPdu::decode(src)?)
                }
                ServerPduType::WireToSurface1 => ServerPdu::WireToSurface1(WireToSurface1Pdu::decode(src)?),
                ServerPduType::WireToSurface2 => ServerPdu::WireToSurface2(WireToSurface2Pdu::decode(src)?),
                ServerPduType::SolidFill => ServerPdu::SolidFill(SolidFillPdu::decode(src)?),
                ServerPduType::SurfaceToSurface => ServerPdu::SurfaceToSurface(SurfaceToSurfacePdu::decode(src)?),
                ServerPduType::SurfaceToCache => ServerPdu::SurfaceToCache(SurfaceToCachePdu::decode(src)?),
                ServerPduType::CacheToSurface => ServerPdu::CacheToSurface(CacheToSurfacePdu::decode(src)?),
                ServerPduType::EvictCacheEntry => ServerPdu::EvictCacheEntry(EvictCacheEntryPdu::decode(src)?),
                ServerPduType::CreateSurface => ServerPdu::CreateSurface(CreateSurfacePdu::decode(src)?),
                ServerPduType::DeleteSurface => ServerPdu::DeleteSurface(DeleteSurfacePdu::decode(src)?),
                ServerPduType::StartFrame => ServerPdu::StartFrame(StartFramePdu::decode(src)?),
                ServerPduType::EndFrame => ServerPdu::EndFrame(EndFramePdu::decode(src)?),
                ServerPduType::ResetGraphics => ServerPdu::ResetGraphics(ResetGraphicsPdu::decode(src)?),
                ServerPduType::MapSurfaceToOutput => ServerPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu::decode(src)?),
                ServerPduType::CapabilitiesConfirm => {
                    ServerPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::decode(src)?)
                }
                ServerPduType::CacheImportReply => ServerPdu::CacheImportReply(CacheImportReplyPdu::decode(src)?),
                ServerPduType::MapSurfaceToScaledOutput => {
                    ServerPdu::MapSurfaceToScaledOutput(MapSurfaceToScaledOutputPdu::decode(src)?)
                }
                ServerPduType::MapSurfaceToScaledWindow => {
                    ServerPdu::MapSurfaceToScaledWindow(MapSurfaceToScaledWindowPdu::decode(src)?)
                }
                _ => return Err(invalid_message_err!("pduType", "invalid pdu type")),
            };
            let buffer_length = pdu.size();

            (pdu, buffer_length)
        };

        if buffer_length != pdu_length {
            Err(invalid_message_err!("len", "invalid pdu length"))
        } else {
            Ok(server_pdu)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPdu {
    FrameAcknowledge(FrameAcknowledgePdu),
    CapabilitiesAdvertise(CapabilitiesAdvertisePdu),
}

impl ClientPdu {
    const NAME: &'static str = "GfxClientPdu";

    const FIXED_PART_SIZE: usize = RDP_GFX_HEADER_SIZE;
}

impl PduEncode for ClientPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(ClientPduType::from(self).to_u16().unwrap());
        dst.write_u16(0); // flags
        dst.write_u32(cast_length!("bufferLen", self.size())?);

        match self {
            ClientPdu::FrameAcknowledge(pdu) => pdu.encode(dst),
            ClientPdu::CapabilitiesAdvertise(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            + match self {
                ClientPdu::FrameAcknowledge(pdu) => pdu.size(),
                ClientPdu::CapabilitiesAdvertise(pdu) => pdu.size(),
            }
    }
}

impl<'a> PduDecode<'a> for ClientPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        let pdu_type = ClientPduType::from_u16(src.read_u16())
            .ok_or_else(|| invalid_message_err!("clientPduType", "invalid pdu type"))?;
        let _flags = src.read_u16();
        let pdu_length = cast_length!("bufferLen", src.read_u32())?;

        let client_pdu = match pdu_type {
            ClientPduType::FrameAcknowledge => ClientPdu::FrameAcknowledge(FrameAcknowledgePdu::decode(src)?),
            ClientPduType::CapabilitiesAdvertise => {
                ClientPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::decode(src)?)
            }
            _ => return Err(invalid_message_err!("pduType", "invalid pdu type")),
        };

        if client_pdu.size() != pdu_length {
            Err(invalid_message_err!("len", "invalid pdu length"))
        } else {
            Ok(client_pdu)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ClientPduType {
    FrameAcknowledge = 0x0d,
    CacheImportOffer = 0x10,
    CapabilitiesAdvertise = 0x12,
    QoeFrameAcknowledge = 0x16,
}

impl<'a> From<&'a ClientPdu> for ClientPduType {
    fn from(c: &'a ClientPdu) -> Self {
        match c {
            ClientPdu::FrameAcknowledge(_) => Self::FrameAcknowledge,
            ClientPdu::CapabilitiesAdvertise(_) => Self::CapabilitiesAdvertise,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ServerPduType {
    WireToSurface1 = 0x01,
    WireToSurface2 = 0x02,
    DeleteEncodingContext = 0x03,
    SolidFill = 0x04,
    SurfaceToSurface = 0x05,
    SurfaceToCache = 0x06,
    CacheToSurface = 0x07,
    EvictCacheEntry = 0x08,
    CreateSurface = 0x09,
    DeleteSurface = 0x0a,
    StartFrame = 0x0b,
    EndFrame = 0x0c,
    ResetGraphics = 0x0e,
    MapSurfaceToOutput = 0x0f,
    CacheImportReply = 0x11,
    CapabilitiesConfirm = 0x13,
    MapSurfaceToWindow = 0x15,
    MapSurfaceToScaledOutput = 0x17,
    MapSurfaceToScaledWindow = 0x18,
}

impl<'a> From<&'a ServerPdu> for ServerPduType {
    fn from(s: &'a ServerPdu) -> Self {
        match s {
            ServerPdu::WireToSurface1(_) => Self::WireToSurface1,
            ServerPdu::WireToSurface2(_) => Self::WireToSurface2,
            ServerPdu::DeleteEncodingContext(_) => Self::DeleteEncodingContext,
            ServerPdu::SolidFill(_) => Self::SolidFill,
            ServerPdu::SurfaceToSurface(_) => Self::SurfaceToSurface,
            ServerPdu::SurfaceToCache(_) => Self::SurfaceToCache,
            ServerPdu::CacheToSurface(_) => Self::CacheToSurface,
            ServerPdu::EvictCacheEntry(_) => Self::EvictCacheEntry,
            ServerPdu::CreateSurface(_) => Self::CreateSurface,
            ServerPdu::DeleteSurface(_) => Self::DeleteSurface,
            ServerPdu::StartFrame(_) => Self::StartFrame,
            ServerPdu::EndFrame(_) => Self::EndFrame,
            ServerPdu::ResetGraphics(_) => Self::ResetGraphics,
            ServerPdu::MapSurfaceToOutput(_) => Self::MapSurfaceToOutput,
            ServerPdu::MapSurfaceToScaledOutput(_) => Self::MapSurfaceToScaledOutput,
            ServerPdu::MapSurfaceToScaledWindow(_) => Self::MapSurfaceToScaledWindow,
            ServerPdu::CapabilitiesConfirm(_) => Self::CapabilitiesConfirm,
            ServerPdu::CacheImportReply(_) => Self::CacheImportReply,
        }
    }
}
