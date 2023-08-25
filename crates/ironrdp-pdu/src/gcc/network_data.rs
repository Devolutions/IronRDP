use std::borrow::Cow;
use std::{io, str};

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_integer::Integer;
use thiserror::Error;

use crate::{try_read_optional, PduParsing};

const CHANNELS_MAX: usize = 31;

const CLIENT_CHANNEL_COUNT_SIZE: usize = 4;

const CLIENT_CHANNEL_OPTIONS_SIZE: usize = 4;
const CLIENT_CHANNEL_SIZE: usize = ChannelName::SIZE + CLIENT_CHANNEL_OPTIONS_SIZE;

const SERVER_IO_CHANNEL_SIZE: usize = 2;
const SERVER_CHANNEL_COUNT_SIZE: usize = 2;
const SERVER_CHANNEL_SIZE: usize = 2;

/// An 8-byte array containing a null-terminated collection of seven ANSI characters
/// with the purpose of uniquely identifying a channel.
///
/// In RDP, an ANSI character is a 8-bit Windows-1252 character set unit. ANSI character set
/// is using all the code values from 0 to 255, as such any u8 value is a valid ANSI character.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelName {
    inner: Cow<'static, [u8; Self::SIZE]>,
}

impl ChannelName {
    pub const SIZE: usize = 8;

    /// Creates a channel name using the provided array, ensuring the last byte is always the null terminator.
    pub const fn new(mut value: [u8; Self::SIZE]) -> Self {
        value[Self::SIZE - 1] = 0; // ensure the last byte is always the null terminator

        Self {
            inner: Cow::Owned(value),
        }
    }

    /// Converts an UTF-8 string into a channel name by copying up to 7 bytes.
    pub fn from_utf8(value: &str) -> Option<Self> {
        let mut inner = [0; Self::SIZE];

        value
            .chars()
            .take(Self::SIZE - 1)
            .zip(inner.iter_mut())
            .try_for_each(|(src, dst)| {
                let c = u8::try_from(src).ok()?;
                c.is_ascii().then(|| *dst = c)
            })?;

        Some(Self {
            inner: Cow::Owned(inner),
        })
    }

    /// Converts a static u8 array into a channel name without copy.
    ///
    /// # Panics
    ///
    /// Panics if input is not null-terminated.
    pub const fn from_static(value: &'static [u8; 8]) -> Self {
        // ensure the last byte is always the null terminator
        if value[Self::SIZE - 1] != 0 {
            panic!("channel name must be null-terminated")
        }

        Self {
            inner: Cow::Borrowed(value),
        }
    }

    /// Returns the underlying raw representation of the channel name (an 8-byte array).
    pub fn as_bytes(&self) -> &[u8; Self::SIZE] {
        self.inner.as_ref()
    }

    /// Get a &str if this channel name is a valid ASCII string.
    pub fn as_str(&self) -> Option<&str> {
        if self.inner.iter().all(u8::is_ascii) {
            let terminator_idx = self
                .inner
                .iter()
                .position(|c| *c == 0)
                .expect("null-terminated ASCII string");
            Some(str::from_utf8(&self.inner[..terminator_idx]).expect("ASCII characters"))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientNetworkData {
    pub channels: Vec<ChannelDef>,
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
            channels.push(ChannelDef::from_buffer(&mut buffer)?);
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

        SERVER_IO_CHANNEL_SIZE + SERVER_CHANNEL_COUNT_SIZE + self.channel_ids.len() * SERVER_CHANNEL_SIZE + padding_size
    }
}

/// Channel Definition Structure (CHANNEL_DEF)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDef {
    pub name: ChannelName,
    pub options: ChannelOptions,
}

impl PduParsing for ChannelDef {
    type Error = NetworkDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mut name = [0; ChannelName::SIZE];
        buffer.read_exact(&mut name)?;
        let name = ChannelName::new(name);

        let options = ChannelOptions::from_bits(buffer.read_u32::<LittleEndian>()?)
            .ok_or(NetworkDataError::InvalidChannelOptions)?;

        Ok(Self { name, options })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_all(self.name.as_bytes())?;
        buffer.write_u32::<LittleEndian>(self.options.bits())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SERVER_CHANNEL_SIZE
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

#[derive(Debug, Error)]
pub enum NetworkDataError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("UTF-8 error")]
    Utf8Error(#[from] str::Utf8Error),
    #[error("Invalid channel options field")]
    InvalidChannelOptions,
    #[error("Invalid channel count field")]
    InvalidChannelCount,
}
