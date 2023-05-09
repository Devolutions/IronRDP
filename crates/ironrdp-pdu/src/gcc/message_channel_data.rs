use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::PduParsing;

const CLIENT_FLAGS_SIZE: usize = 4;
const SERVER_MCS_MESSAGE_CHANNEL_ID_SIZE: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientMessageChannelData;

impl PduParsing for ClientMessageChannelData {
    type Error = io::Error;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let _flags = buffer.read_u32::<LittleEndian>()?; // is unused

        Ok(Self {})
    }
    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(0)?; // flags

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CLIENT_FLAGS_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerMessageChannelData {
    pub mcs_message_channel_id: u16,
}

impl PduParsing for ServerMessageChannelData {
    type Error = io::Error;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mcs_message_channel_id = buffer.read_u16::<LittleEndian>()?;

        Ok(Self { mcs_message_channel_id })
    }
    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.mcs_message_channel_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SERVER_MCS_MESSAGE_CHANNEL_ID_SIZE
    }
}
