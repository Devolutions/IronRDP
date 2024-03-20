mod graphics_messages;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use graphics_messages::RESET_GRAPHICS_PDU_SIZE;
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
use thiserror::Error;

use crate::{PduError, PduParsing};

const RDP_GFX_HEADER_SIZE: usize = 8;

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

        let (server_pdu, buffer_length) = {
            let pdu = match pdu_type {
                ServerPduType::DeleteEncodingContext => {
                    ServerPdu::DeleteEncodingContext(DeleteEncodingContextPdu::from_buffer(&mut stream)?)
                }
                ServerPduType::WireToSurface1 => {
                    ServerPdu::WireToSurface1(WireToSurface1Pdu::from_buffer(&mut stream)?)
                }
                ServerPduType::WireToSurface2 => {
                    ServerPdu::WireToSurface2(WireToSurface2Pdu::from_buffer(&mut stream)?)
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
                ServerPduType::CreateSurface => ServerPdu::CreateSurface(CreateSurfacePdu::from_buffer(&mut stream)?),
                ServerPduType::DeleteSurface => ServerPdu::DeleteSurface(DeleteSurfacePdu::from_buffer(&mut stream)?),
                ServerPduType::StartFrame => ServerPdu::StartFrame(StartFramePdu::from_buffer(&mut stream)?),
                ServerPduType::EndFrame => ServerPdu::EndFrame(EndFramePdu::from_buffer(&mut stream)?),
                ServerPduType::ResetGraphics => ServerPdu::ResetGraphics(ResetGraphicsPdu::from_buffer(&mut stream)?),
                ServerPduType::MapSurfaceToOutput => {
                    ServerPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu::from_buffer(&mut stream)?)
                }
                ServerPduType::CapabilitiesConfirm => {
                    ServerPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_buffer(&mut stream)?)
                }
                ServerPduType::CacheImportReply => {
                    ServerPdu::CacheImportReply(CacheImportReplyPdu::from_buffer(&mut stream)?)
                }
                ServerPduType::MapSurfaceToScaledOutput => {
                    ServerPdu::MapSurfaceToScaledOutput(MapSurfaceToScaledOutputPdu::from_buffer(&mut stream)?)
                }
                ServerPduType::MapSurfaceToScaledWindow => {
                    ServerPdu::MapSurfaceToScaledWindow(MapSurfaceToScaledWindowPdu::from_buffer(&mut stream)?)
                }
                _ => return Err(GraphicsPipelineError::UnexpectedServerPduType(pdu_type)),
            };
            let buffer_length = pdu.buffer_length();

            (pdu, buffer_length)
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
        let buffer_length = self.buffer_length();

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
            ServerPdu::MapSurfaceToScaledOutput(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
            ServerPdu::MapSurfaceToScaledWindow(pdu) => pdu.to_buffer(&mut stream).map_err(GraphicsPipelineError::from),
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
                ServerPdu::MapSurfaceToScaledOutput(pdu) => pdu.buffer_length(),
                ServerPdu::MapSurfaceToScaledWindow(pdu) => pdu.buffer_length(),
                ServerPdu::StartFrame(pdu) => pdu.buffer_length(),
                ServerPdu::EndFrame(pdu) => pdu.buffer_length(),
                ServerPdu::EvictCacheEntry(pdu) => pdu.buffer_length(),
                ServerPdu::CapabilitiesConfirm(pdu) => pdu.buffer_length(),
                ServerPdu::CacheImportReply(pdu) => pdu.buffer_length(),
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Error)]
pub enum GraphicsPipelineError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("graphics messages error")]
    GraphicsMessagesError(#[from] graphics_messages::GraphicsMessagesError),
    #[error("invalid Header cmd ID")]
    InvalidCmdId,
    #[error("unexpected client's PDU type: {0:?}")]
    UnexpectedClientPduType(ClientPduType),
    #[error("unexpected server's PDU type: {0:?}")]
    UnexpectedServerPduType(ServerPduType),
    #[error("invalid ResetGraphics PDU size: expected ({expected}) != actual ({actual})")]
    InvalidResetGraphicsPduSize { expected: usize, actual: usize },
    #[error("invalid PDU length: expected ({expected}) != actual ({actual})")]
    InvalidPduLength { expected: usize, actual: usize },
    #[error("PDU error: {0}")]
    Pdu(PduError),
}

impl From<PduError> for GraphicsPipelineError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}

#[cfg(feature = "std")]
impl ironrdp_error::legacy::ErrorContext for GraphicsPipelineError {
    fn context(&self) -> &'static str {
        "graphics pipeline"
    }
}
