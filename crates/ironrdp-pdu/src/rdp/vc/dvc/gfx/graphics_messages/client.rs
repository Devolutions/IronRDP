use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::{CapabilitySet, GraphicsMessagesError};
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitiesAdvertisePdu(pub Vec<CapabilitySet>);

impl PduParsing for CapabilitiesAdvertisePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let capabilities_count = stream.read_u16::<LittleEndian>()? as usize;

        let capabilities = (0..capabilities_count)
            .map(|_| CapabilitySet::from_buffer(&mut stream))
            .collect::<Result<Vec<_>, Self::Error>>()?;

        Ok(Self(capabilities))
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.0.len() as u16)?;

        for capability_set in self.0.iter() {
            capability_set.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        2 + self.0.iter().map(|c| c.buffer_length()).sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameAcknowledgePdu {
    pub queue_depth: QueueDepth,
    pub frame_id: u32,
    pub total_frames_decoded: u32,
}

impl PduParsing for FrameAcknowledgePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let queue_depth = QueueDepth::from_u32(stream.read_u32::<LittleEndian>()?);
        let frame_id = stream.read_u32::<LittleEndian>()?;
        let total_frames_decoded = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            queue_depth,
            frame_id,
            total_frames_decoded,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.queue_depth.to_u32())?;
        stream.write_u32::<LittleEndian>(self.frame_id)?;
        stream.write_u32::<LittleEndian>(self.total_frames_decoded)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        12
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheImportReplyPdu {
    pub cache_slots: Vec<u16>,
}

impl PduParsing for CacheImportReplyPdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let entries_count = stream.read_u16::<LittleEndian>()? as usize;

        let cache_slots = (0..entries_count)
            .map(|_| stream.read_u16::<LittleEndian>())
            .collect::<io::Result<Vec<_>>>()?;

        Ok(Self { cache_slots })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.cache_slots.len() as u16)?;

        for cache_slot in self.cache_slots.iter() {
            stream.write_u16::<LittleEndian>(*cache_slot)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        2 + self.cache_slots.iter().map(|_| 2).sum::<usize>()
    }
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum QueueDepth {
    Unavailable,
    AvailableBytes(u32),
    Suspend,
}

impl QueueDepth {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0x0000_0000 => Self::Unavailable,
            0x0000_0001..=0xFFFF_FFFE => Self::AvailableBytes(v),
            0xFFFF_FFFF => Self::Suspend,
        }
    }

    pub fn to_u32(self) -> u32 {
        match self {
            Self::Unavailable => 0x0000_0000,
            Self::AvailableBytes(v) => v,
            Self::Suspend => 0xFFFF_FFFF,
        }
    }
}
