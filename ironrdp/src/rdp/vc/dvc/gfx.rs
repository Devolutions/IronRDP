pub mod zgfx;

mod graphics_messages;
#[cfg(test)]
mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use graphics_messages::RESET_GRAPHICS_PDU_SIZE;
pub use graphics_messages::{
    CacheImportReplyPdu, CacheToSurfacePdu, CapabilitiesAdvertisePdu, CapabilitiesConfirmPdu, CapabilitiesV103Flags,
    CapabilitiesV104Flags, CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV8Flags, CapabilitySet, Codec1Type,
    Codec2Type, CreateSurfacePdu, DeleteEncodingContextPdu, DeleteSurfacePdu, EndFramePdu, EvictCacheEntryPdu,
    FrameAcknowledgePdu, MapSurfaceToOutputPdu, PixelFormat, QueueDepth, ResetGraphicsPdu, SolidFillPdu, StartFramePdu,
    SurfaceToCachePdu, SurfaceToSurfacePdu, WireToSurface1Pdu, WireToSurface2Pdu,
};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{impl_from_error, PduParsing};

const RDP_GFX_HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq)]
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
}

impl PduParsing for ServerPdu {
    type Error = GraphicsPipelineError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let pdu_type =
            ServerPduType::from_u16(stream.read_u16::<LittleEndian>()?).ok_or(GraphicsPipelineError::InvalidCmdId)?;
        let _flags = stream.read_u16::<LittleEndian>()?;
        let pdu_length = stream.read_u32::<LittleEndian>()? as usize;

        if let ServerPduType::ResetGraphics = pdu_type {
            if pdu_length != RESET_GRAPHICS_PDU_SIZE {
                return Err(GraphicsPipelineError::InvalidResetGraphicsPduSize {
                    expected: RESET_GRAPHICS_PDU_SIZE,
                    actual: pdu_length,
                });
            }
        }

        let (server_pdu, buffer_length) = match pdu_type {
            ServerPduType::WireToSurface1 => {
                let pdu = WireToSurface1Pdu::from_buffer(&mut stream)?;
                let bitmap_data_length = pdu.bitmap_data_length;

                let pdu = ServerPdu::WireToSurface1(pdu);
                let buffer_length = pdu.buffer_length() + bitmap_data_length;

                (pdu, buffer_length)
            }
            ServerPduType::WireToSurface2 => {
                let pdu = WireToSurface2Pdu::from_buffer(&mut stream)?;
                let bitmap_data_length = pdu.bitmap_data_length;

                let pdu = ServerPdu::WireToSurface2(pdu);
                let buffer_length = pdu.buffer_length() + bitmap_data_length;

                (pdu, buffer_length)
            }
            _ => {
                let pdu = match pdu_type {
                    ServerPduType::DeleteEncodingContext => {
                        ServerPdu::DeleteEncodingContext(DeleteEncodingContextPdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::SolidFill => ServerPdu::SolidFill(SolidFillPdu::from_buffer(&mut stream)?),
                    ServerPduType::SurfaceToSurface => {
                        ServerPdu::SurfaceToSurface(SurfaceToSurfacePdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::SurfaceToCache => {
                        ServerPdu::SurfaceToCache(SurfaceToCachePdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::CacheToSurface => {
                        ServerPdu::CacheToSurface(CacheToSurfacePdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::EvictCacheEntry => {
                        ServerPdu::EvictCacheEntry(EvictCacheEntryPdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::CreateSurface => {
                        ServerPdu::CreateSurface(CreateSurfacePdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::DeleteSurface => {
                        ServerPdu::DeleteSurface(DeleteSurfacePdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::StartFrame => ServerPdu::StartFrame(StartFramePdu::from_buffer(&mut stream)?),
                    ServerPduType::EndFrame => ServerPdu::EndFrame(EndFramePdu::from_buffer(&mut stream)?),
                    ServerPduType::ResetGraphics => {
                        ServerPdu::ResetGraphics(ResetGraphicsPdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::MapSurfaceToOutput => {
                        ServerPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::CapabilitiesConfirm => {
                        ServerPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::CacheImportReply => {
                        ServerPdu::CacheImportReply(CacheImportReplyPdu::from_buffer(&mut stream)?)
                    }
                    ServerPduType::WireToSurface1 | ServerPduType::WireToSurface2 => unreachable!(),
                    _ => return Err(GraphicsPipelineError::UnexpectedServerPduType(pdu_type)),
                };
                let buffer_length = pdu.buffer_length();

                (pdu, buffer_length)
            }
        };

        if buffer_length != pdu_length {
            Err(GraphicsPipelineError::InvalidPduLength {
                expected: pdu_length,
                actual: buffer_length,
            })
        } else {
            Ok(server_pdu)
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let buffer_length = match self {
            ServerPdu::WireToSurface1(pdu) => self.buffer_length() + pdu.bitmap_data_length,
            ServerPdu::WireToSurface2(pdu) => self.buffer_length() + pdu.bitmap_data_length,
            _ => self.buffer_length(),
        };

        stream.write_u16::<LittleEndian>(ServerPduType::from(self).to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(0)?; // flags
        stream.write_u32::<LittleEndian>(buffer_length as u32)?;

        match self {
            ServerPdu::WireToSurface1(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::WireToSurface2(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::DeleteEncodingContext(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::SolidFill(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::SurfaceToSurface(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::SurfaceToCache(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::CacheToSurface(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::CreateSurface(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::DeleteSurface(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::ResetGraphics(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::MapSurfaceToOutput(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::StartFrame(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::EndFrame(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::EvictCacheEntry(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::CapabilitiesConfirm(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::CacheImportReply(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
        }
    }

    fn buffer_length(&self) -> usize {
        RDP_GFX_HEADER_SIZE
            + match self {
                ServerPdu::WireToSurface1(pdu) => pdu.buffer_length(),
                ServerPdu::WireToSurface2(pdu) => pdu.buffer_length(),
                ServerPdu::DeleteEncodingContext(pdu) => pdu.buffer_length(),
                ServerPdu::SolidFill(pdu) => pdu.buffer_length(),
                ServerPdu::SurfaceToSurface(pdu) => pdu.buffer_length(),
                ServerPdu::SurfaceToCache(pdu) => pdu.buffer_length(),
                ServerPdu::CacheToSurface(pdu) => pdu.buffer_length(),
                ServerPdu::CreateSurface(pdu) => pdu.buffer_length(),
                ServerPdu::DeleteSurface(pdu) => pdu.buffer_length(),
                ServerPdu::ResetGraphics(pdu) => pdu.buffer_length(),
                ServerPdu::MapSurfaceToOutput(pdu) => pdu.buffer_length(),
                ServerPdu::StartFrame(pdu) => pdu.buffer_length(),
                ServerPdu::EndFrame(pdu) => pdu.buffer_length(),
                ServerPdu::EvictCacheEntry(pdu) => pdu.buffer_length(),
                ServerPdu::CapabilitiesConfirm(pdu) => pdu.buffer_length(),
                ServerPdu::CacheImportReply(pdu) => pdu.buffer_length(),
            }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientPdu {
    FrameAcknowledge(FrameAcknowledgePdu),
    CapabilitiesAdvertise(CapabilitiesAdvertisePdu),
}

impl PduParsing for ClientPdu {
    type Error = GraphicsPipelineError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let pdu_type =
            ClientPduType::from_u16(stream.read_u16::<LittleEndian>()?).ok_or(GraphicsPipelineError::InvalidCmdId)?;
        let _flags = stream.read_u16::<LittleEndian>()?;
        let pdu_length = stream.read_u32::<LittleEndian>()? as usize;

        let client_pdu = match pdu_type {
            ClientPduType::FrameAcknowledge => {
                ClientPdu::FrameAcknowledge(FrameAcknowledgePdu::from_buffer(&mut stream)?)
            }
            ClientPduType::CapabilitiesAdvertise => {
                ClientPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_buffer(&mut stream)?)
            }
            _ => return Err(GraphicsPipelineError::UnexpectedClientPduType(pdu_type)),
        };

        if client_pdu.buffer_length() != pdu_length {
            Err(GraphicsPipelineError::InvalidPduLength {
                expected: pdu_length,
                actual: client_pdu.buffer_length(),
            })
        } else {
            Ok(client_pdu)
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(ClientPduType::from(self).to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(0)?; // flags
        stream.write_u32::<LittleEndian>(self.buffer_length() as u32)?;

        match self {
            ClientPdu::FrameAcknowledge(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ClientPdu::CapabilitiesAdvertise(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
        }
    }

    fn buffer_length(&self) -> usize {
        RDP_GFX_HEADER_SIZE
            + match self {
                ClientPdu::FrameAcknowledge(pdu) => pdu.buffer_length(),
                ClientPdu::CapabilitiesAdvertise(pdu) => pdu.buffer_length(),
            }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
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

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
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
            ServerPdu::CapabilitiesConfirm(_) => Self::CapabilitiesConfirm,
            ServerPdu::CacheImportReply(_) => Self::CacheImportReply,
        }
    }
}

#[derive(Debug, Fail)]
pub enum GraphicsPipelineError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Graphics messages error: {}", _0)]
    GraphicsMessagesError(#[fail(cause)] graphics_messages::GraphicsMessagesError),
    #[fail(display = "Invalid Header cmd ID")]
    InvalidCmdId,
    #[fail(display = "Unexpected client's PDU type: {:?}", _0)]
    UnexpectedClientPduType(ClientPduType),
    #[fail(display = "Unexpected server's PDU type: {:?}", _0)]
    UnexpectedServerPduType(ServerPduType),
    #[fail(
        display = "Invalid ResetGraphics PDU size: expected ({}) != actual ({})",
        expected, actual
    )]
    InvalidResetGraphicsPduSize { expected: usize, actual: usize },
    #[fail(display = "Invalid PDU length: expected ({}) != actual ({})", expected, actual)]
    InvalidPduLength { expected: usize, actual: usize },
}

impl_from_error!(io::Error, GraphicsPipelineError, GraphicsPipelineError::IOError);

impl_from_error!(
    graphics_messages::GraphicsMessagesError,
    GraphicsPipelineError,
    GraphicsPipelineError::GraphicsMessagesError
);
