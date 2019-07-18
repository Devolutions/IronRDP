#[cfg(test)]
pub mod test;

use std::{
    io::{self, Write},
    str,
};

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_integer::Integer;

use crate::PduParsing;

const CHANNELS_MAX: usize = 31;

const CLIENT_CHANNEL_COUNT_SIZE: usize = 4;

const CLIENT_CHANNEL_NAME_SIZE: usize = 8;
const CLIENT_CHANNEL_OPTIONS_SIZE: usize = 4;
const CLIENT_CHANNEL_SIZE: usize = CLIENT_CHANNEL_NAME_SIZE + CLIENT_CHANNEL_OPTIONS_SIZE;

const SERVER_IO_CHANNEL_SIZE: usize = 2;
const SERVER_CHANNEL_COUNT_SIZE: usize = 2;
const SERVER_CHANNEL_SIZE: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientNetworkData {
    pub channels: Vec<Channel>,
}

impl PduParsing for ClientNetworkData {
    type Error = NetworkDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let channel_count = buffer.read_u32::<LittleEndian>()?;

        if channel_count > CHANNELS_MAX as u32 {
            return Err(NetworkDataError::InvalidChannelCount);
        }

        let mut channels = Vec::with_capacity(channel_count as usize);
        for _ in 0..channel_count {
            channels.push(Channel::from_buffer(&mut buffer)?);
        }

        Ok(Self { channels })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.channels.len() as u32)?;

        for channel in self.channels.iter().take(CHANNELS_MAX) {
            channel.to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CLIENT_CHANNEL_COUNT_SIZE + self.channels.len() * CLIENT_CHANNEL_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerNetworkData {
    pub channel_ids: Vec<u16>,
    pub io_channel: u16,
}

impl ServerNetworkData {
    fn write_padding(&self) -> bool {
        self.channel_ids.len().is_odd()
    }
}

impl PduParsing for ServerNetworkData {
    type Error = NetworkDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let io_channel = buffer.read_u16::<LittleEndian>()?;
        let channel_count = buffer.read_u16::<LittleEndian>()?;

        let mut channel_ids = Vec::with_capacity(channel_count as usize);
        for _ in 0..channel_count {
            channel_ids.push(buffer.read_u16::<LittleEndian>()?);
        }

        let result = Self {
            io_channel,
            channel_ids,
        };

        let _pad = try_read_optional!(buffer.read_u16::<LittleEndian>(), result);

        Ok(result)
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.io_channel)?;
        buffer.write_u16::<LittleEndian>(self.channel_ids.len() as u16)?;

        for channel_id in self.channel_ids.iter() {
            buffer.write_u16::<LittleEndian>(*channel_id)?;
        }

        // The size in bytes of the Server Network Data structure MUST be a multiple of 4.
        // If the channelCount field contains an odd value, then the size of the channelIdArray
        // (and by implication the entire Server Network Data structure) will not be a multiple of 4.
        // In this scenario, the Pad field MUST be present and it is used to add an additional
        // 2 bytes to the size of the Server Network Data structure.
        if self.write_padding() {
            buffer.write_u16::<LittleEndian>(0)?; // pad
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let padding_size = if self.write_padding() { 2 } else { 0 };

        SERVER_IO_CHANNEL_SIZE
            + SERVER_CHANNEL_COUNT_SIZE
            + self.channel_ids.len() * SERVER_CHANNEL_SIZE
            + padding_size
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub name: String,
    options: ChannelOptions,
}

impl PduParsing for Channel {
    type Error = NetworkDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mut name = [0; CLIENT_CHANNEL_NAME_SIZE];
        buffer.read_exact(&mut name)?;
        let name = str::from_utf8(name.as_ref())
            .map_err(NetworkDataError::Utf8Error)?
            .trim_end_matches('\u{0}')
            .into();
        let options = ChannelOptions::from_bits(buffer.read_u32::<LittleEndian>()?)
            .ok_or(NetworkDataError::InvalidChannelOptions)?;

        Ok(Self { name, options })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        let mut name = [0; CLIENT_CHANNEL_NAME_SIZE - 1];
        name.as_mut().write_all(self.name.as_bytes())?;

        buffer.write_all(name.as_ref())?;
        buffer.write_u8(0)?; // null-terminated
        buffer.write_u32::<LittleEndian>(self.options.bits())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SERVER_CHANNEL_SIZE
    }
}

bitflags! {
    pub struct ChannelOptions: u32 {
        const INITIALIZED = 0x8000_0000;
        const ENCRYPT_RDP = 0x4000_0000;
        const ENCRYPT_SC = 0x2000_0000;
        const ENCRYPT_CS = 0x1000_0000;
        const PRI_HIGH = 0x0800_0000;
        const PRI_MED = 0x0400_0000;
        const PRI_LOW = 0x0200_0000;
        const COMPRESS_RDP = 0x0080_0000;
        const COMPRESS = 0x0040_0000;
        const SHOW_PROTOCOL = 0x0020_0000;
        const REMOTE_CONTROL_PERSISTENT = 0x0010_0000;
    }
}

#[derive(Debug, Fail)]
pub enum NetworkDataError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "UTF-8 error: {}", _0)]
    Utf8Error(#[fail(cause)] str::Utf8Error),
    #[fail(display = "Invalid channel options field")]
    InvalidChannelOptions,
    #[fail(display = "Invalid channel count field")]
    InvalidChannelCount,
}

impl_from_error!(io::Error, NetworkDataError, NetworkDataError::IOError);
impl_from_error!(
    str::Utf8Error,
    NetworkDataError,
    NetworkDataError::Utf8Error
);
