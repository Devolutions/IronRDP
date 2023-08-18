extern crate alloc;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use bitflags::bitflags;
use core::any::{Any, TypeId};
use core::fmt;
use pdu::cursor::WriteCursor;
use pdu::{encode_buf, PduEncode};

use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{assert_obj_safe, PduResult};
use pdu::gcc::Channel;

pub type StaticChannelId = u16;

/// Defines which compression flag should be sent along the Channel Definition Structure (CHANNEL_DEF)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionCondition {
    /// Virtual channel data will not be compressed
    Never,
    /// Virtual channel data MUST be compressed if RDP data is being compressed (CHANNEL_OPTION_COMPRESS_RDP)
    WhenRdpDataIsCompressed,
    /// Virtual channel data MUST be compressed, regardless of RDP compression settings (CHANNEL_OPTION_COMPRESS)
    Always,
}

/// A type that is a Static Virtual Channel
///
/// Static virtual channels are created once at the beginning of the RDP session and allow lossless
/// communication between client and server components over the main data connection.
/// There are at most 31 (optional) static virtual channels that can be created for a single connection, for a
/// total of 32 static channels when accounting for the non-optional I/O channel.
pub trait StaticVirtualChannel: AsAny + fmt::Debug + Send + Sync {
    /// Returns the name of the `StaticVirtualChannel`
    fn channel_name(&self) -> ChannelName;

    /// Defines which compression flag should be sent along the Channel Definition Structure (CHANNEL_DEF)
    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    /// Processes a payload received on the virtual channel.
    /// Implementer is responsible for filling outputs with
    /// each fully encoded PDU to be sent back to the client.
    ///
    /// They are NOT repsonsible for splitting the PDU into
    /// multiple chunks this is handled by the `StaticVirtualChannelCodec`.
    fn process(
        &mut self,
        initiator_id: StaticChannelId,
        channel_id: StaticChannelId,
        payload: &[u8],
        outputs: &mut [WriteBuf; 2],
    ) -> PduResult<()>;

    #[doc(hidden)]
    fn is_drdynvc(&self) -> bool {
        // FIXME: temporary method that will be removed once drdynvc is ported to the new API
        false
    }
}

/// Takes an array of individual, fully-encoded PDUs (created by a call to [`StaticVirtualChannel::process`])
/// and breaks them into chunks prefixed with a [`ChannelPDUHeader`]. Each chunk is at most `max_chunk_len`
/// bytes long (not including the ChannelPduHeader).
pub fn chunkify(encoded_pdus: &mut [WriteBuf; 2], max_chunk_len: usize) -> PduResult<Vec<WriteBuf>> {
    let mut results = Vec::new();

    for encoded_pdu in encoded_pdus {
        let chunks = chunkify_one(encoded_pdu, max_chunk_len)?;
        results.extend(chunks);
    }

    Ok(results)
}

/// See [`chunkify`].
fn chunkify_one(encoded_pdu: &mut WriteBuf, max_chunk_len: usize) -> PduResult<Vec<WriteBuf>> {
    let mut chunks = Vec::new();

    let total_len = encoded_pdu.filled_len();
    let mut chunk_start_index: usize = 0;
    let mut chunk_end_index = std::cmp::min(total_len, max_chunk_len);
    loop {
        // Create a buffer to hold this next chunk.
        let mut chunk = WriteBuf::new();

        // Set the first and last flags if this is the first and/or last chunk for this PDU.
        let first = chunk_start_index == 0;
        let last = chunk_end_index == total_len;

        // Create the header for this chunk.
        let header = {
            let mut flags = ChannelPDUFlags::empty();
            if first {
                flags |= ChannelPDUFlags::CHANNEL_FLAG_FIRST;
            }
            if last {
                flags |= ChannelPDUFlags::CHANNEL_FLAG_LAST;
            }

            ChannelPDUHeader {
                length: total_len as u32,
                flags,
            }
        };

        // Encode the header for this chunk.
        encode_buf(&header, &mut chunk)?;
        // Append the piece of the encoded_pdu that belongs in this chunk.
        chunk.write_slice(encoded_pdu.slice(chunk_start_index, chunk_end_index));
        // Push the chunk onto the results.
        chunks.push(chunk);

        // If this was the last chunk, we're done, return the results.
        if last {
            break;
        }

        // Otherwise, update the chunk start and end indices for the next iteration.
        chunk_start_index = chunk_end_index;
        chunk_end_index = std::cmp::min(total_len, chunk_end_index + max_chunk_len);
    }

    Ok(chunks)
}

assert_obj_safe!(StaticVirtualChannel);

/// Build the `ChannelOptions` bitfield to be used in the Channel Definition Structure.
pub fn make_channel_options(channel: &dyn StaticVirtualChannel) -> ChannelOptions {
    match channel.compression_condition() {
        CompressionCondition::Never => ChannelOptions::empty(),
        CompressionCondition::WhenRdpDataIsCompressed => ChannelOptions::COMPRESS_RDP,
        CompressionCondition::Always => ChannelOptions::COMPRESS,
    }
}

/// Build the Channel Definition Structure (CHANNEL_DEF) containing information for this channel.
pub fn make_channel_definition(channel: &dyn StaticVirtualChannel) -> Channel {
    let name = channel.channel_name();
    let options = make_channel_options(channel);
    Channel { name, options }
}

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Debug)]
pub struct StaticChannelSet {
    channels: BTreeMap<TypeId, Box<dyn StaticVirtualChannel>>,
    to_channel_id: BTreeMap<TypeId, StaticChannelId>,
    to_type_id: BTreeMap<StaticChannelId, TypeId>,
}

impl StaticChannelSet {
    #[inline]
    pub fn new() -> Self {
        Self {
            channels: BTreeMap::new(),
            to_channel_id: BTreeMap::new(),
            to_type_id: BTreeMap::new(),
        }
    }

    pub fn insert<T: StaticVirtualChannel + 'static>(&mut self, val: T) -> Option<Box<dyn StaticVirtualChannel>> {
        self.channels.insert(TypeId::of::<T>(), Box::new(val))
    }

    pub fn get_by_type_id(&self, type_id: TypeId) -> Option<&dyn StaticVirtualChannel> {
        self.channels.get(&type_id).map(|boxed| boxed.as_ref())
    }

    pub fn get_by_type_id_mut(&mut self, type_id: TypeId) -> Option<&mut dyn StaticVirtualChannel> {
        if let Some(boxed) = self.channels.get_mut(&type_id) {
            Some(boxed.as_mut())
        } else {
            None
        }
    }

    pub fn get_by_type<T: StaticVirtualChannel + 'static>(&self) -> Option<&dyn StaticVirtualChannel> {
        self.get_by_type_id(TypeId::of::<T>())
    }

    pub fn get_by_type_mut<T: StaticVirtualChannel + 'static>(&mut self) -> Option<&mut dyn StaticVirtualChannel> {
        self.get_by_type_id_mut(TypeId::of::<T>())
    }

    pub fn get_by_channel_id(&self, channel_id: StaticChannelId) -> Option<&dyn StaticVirtualChannel> {
        self.get_type_id_by_channel_id(channel_id)
            .and_then(|type_id| self.get_by_type_id(type_id))
    }

    pub fn get_by_channel_id_mut(&mut self, channel_id: StaticChannelId) -> Option<&mut dyn StaticVirtualChannel> {
        self.get_type_id_by_channel_id(channel_id)
            .and_then(|type_id| self.get_by_type_id_mut(type_id))
    }

    pub fn remove_by_type<T: StaticVirtualChannel + 'static>(&mut self) -> Option<Box<dyn StaticVirtualChannel>> {
        self.channels.remove(&TypeId::of::<T>())
    }

    pub fn attach_channel_id(&mut self, type_id: TypeId, channel_id: StaticChannelId) -> Option<StaticChannelId> {
        self.to_type_id.insert(channel_id, type_id);
        self.to_channel_id.insert(type_id, channel_id)
    }

    pub fn get_channel_id_by_type_id(&self, type_id: TypeId) -> Option<StaticChannelId> {
        self.to_channel_id.get(&type_id).copied()
    }

    pub fn get_channel_id_by_type<T: StaticVirtualChannel + 'static>(&self) -> Option<StaticChannelId> {
        self.get_channel_id_by_type_id(TypeId::of::<T>())
    }

    pub fn get_type_id_by_channel_id(&self, channel_id: StaticChannelId) -> Option<TypeId> {
        self.to_type_id.get(&channel_id).copied()
    }

    pub fn detach_id(&mut self, type_id: TypeId) -> Option<StaticChannelId> {
        if let Some(channel_id) = self.to_channel_id.remove(&type_id) {
            self.to_type_id.remove(&channel_id);
            Some(channel_id)
        } else {
            None
        }
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (TypeId, &dyn StaticVirtualChannel)> {
        self.channels.iter().map(|(type_id, boxed)| (*type_id, boxed.as_ref()))
    }

    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &dyn StaticVirtualChannel> {
        self.channels.values().map(|boxed| boxed.as_ref())
    }

    #[inline]
    pub fn type_ids(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.channels.keys().copied()
    }

    #[inline]
    pub fn channel_ids(&self) -> impl Iterator<Item = StaticChannelId> + '_ {
        self.to_channel_id.values().copied()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.channels.clear();
        self.to_channel_id.clear();
        self.to_type_id.clear();
    }

    #[inline]
    pub fn take(&mut self) -> Self {
        Self {
            channels: core::mem::take(&mut self.channels),
            to_channel_id: core::mem::take(&mut self.to_channel_id),
            to_type_id: core::mem::take(&mut self.to_type_id),
        }
    }
}

impl Default for StaticChannelSet {
    fn default() -> Self {
        Self::new()
    }
}

/// The default maximum chunk size for virtual channel data.
///
/// If an RDP server supports larger chunks, it will advertise
/// the larger chunk size in the `VCChunkSize` field of the
/// virtual channel capability set.
///
/// See also:
/// - https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/6c074267-1b32-4ceb-9496-2eb941a23e6b
/// - https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/a8593178-80c0-4b80-876c-cb77e62cecfc
pub const CHANNEL_CHUNK_LEGNTH: usize = 1600;

bitflags! {
    /// Channel control flags, as specified in section 2.2.6.1.1 of MS-RDPBCGR.
    ///
    /// See: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/f125c65e-6901-43c3-8071-d7d5aaee7ae4
    #[derive(Debug, PartialEq, Copy, Clone)]
    struct ChannelPDUFlags: u32 {
        const CHANNEL_FLAG_FIRST = 0x00000001;
        const CHANNEL_FLAG_LAST = 0x00000002;
        const CHANNEL_FLAG_SHOW_PROTOCOL = 0x00000010;
        const CHANNEL_FLAG_SUSPEND = 0x00000020;
        const CHANNEL_FLAG_RESUME = 0x00000040;
        const CHANNEL_FLAG_SHADOW_PERSISTENT = 0x00000080;
        const CHANNEL_PACKET_COMPRESSED = 0x00200000;
        const CHANNEL_PACKET_AT_FRONT = 0x00400000;
        const CHANNEL_PACKET_FLUSHED = 0x00800000;

        const CHANNEL_FLAG_ONLY = Self::CHANNEL_FLAG_FIRST.bits() | Self::CHANNEL_FLAG_LAST.bits();
    }
}

/// Channel PDU header precedes all static virtual channel traffic
/// transmitted between an RDP client and server.
///
/// It is specified in section 2.2.6.1.1 of MS-RDPBCGR.
/// https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/f125c65e-6901-43c3-8071-d7d5aaee7ae4
#[derive(Debug)]
struct ChannelPDUHeader {
    /// The total length of the uncompressed PDU data,
    /// excluding the length of this header.
    /// Note: the data can span multiple PDUs, in which
    /// case each PDU in the series contains the same
    /// length field.
    length: u32,
    flags: ChannelPDUFlags,
}

impl PduEncode for ChannelPDUHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u32(self.length);
        dst.write_u32(self.flags.bits());
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ChannelPDUHeader"
    }

    fn size(&self) -> usize {
        std::mem::size_of::<u32>() * 2
    }
}
