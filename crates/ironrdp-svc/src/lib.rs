#![doc = include_str!("../README.md")]
// TODO: #![warn(missing_docs)]

extern crate alloc;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::any::TypeId;
use core::fmt;
use std::borrow::Cow;
use std::marker::PhantomData;

use bitflags::bitflags;
use ironrdp_core::assert_obj_safe;
use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{mcs, PduResult};
use pdu::gcc::ChannelDef;
use pdu::rdp::vc::ChannelControlFlags;
use pdu::{decode_cursor, encode_buf, PduEncode};

/// The integer type representing a static virtual channel ID.
pub type StaticChannelId = u16;

/// SVC data to be sent to the server. See [`SvcMessage`] for more information.
/// Usually returned by the channel-specific methods.
pub struct SvcProcessorMessages<P: SvcProcessor> {
    messages: Vec<SvcMessage>,
    _channel: PhantomData<P>,
}

impl<P: SvcProcessor> SvcProcessorMessages<P> {
    pub fn new(messages: Vec<SvcMessage>) -> Self {
        Self {
            messages,
            _channel: PhantomData,
        }
    }
}

impl<P: SvcProcessor> From<Vec<SvcMessage>> for SvcProcessorMessages<P> {
    fn from(messages: Vec<SvcMessage>) -> Self {
        Self::new(messages)
    }
}

impl<P: SvcProcessor> From<SvcProcessorMessages<P>> for Vec<SvcMessage> {
    fn from(request: SvcProcessorMessages<P>) -> Self {
        request.messages
    }
}

/// Represents a message that, when encoded, forms a complete PDU for a given static virtual channel, sans any [`ChannelPduHeader`].
/// In other words, this marker should be applied to a message that is ready to be chunkified (have [`ChannelPduHeader`]s added,
/// splitting it into chunks if necessary) and wrapped in MCS, x224, and tpkt headers for sending over the wire.
pub trait SvcPduEncode: PduEncode + Send {}

/// For legacy reasons, we implement [`SvcPduEncode`] for [`Vec<u8>`].
// FIXME: legacy code
impl SvcPduEncode for Vec<u8> {}

/// Encodable PDU to be sent over a static virtual channel.
///
/// Additional SVC header flags can be added via [`SvcMessage::with_flags`] method.
pub struct SvcMessage {
    pdu: Box<dyn SvcPduEncode>,
    flags: ChannelFlags,
}

impl SvcMessage {
    /// Adds additional SVC header flags to the message.
    #[must_use]
    pub fn with_flags(mut self, flags: ChannelFlags) -> Self {
        self.flags |= flags;
        self
    }
}

impl<T> From<T> for SvcMessage
where
    T: SvcPduEncode + 'static,
{
    fn from(pdu: T) -> Self {
        Self {
            pdu: Box::new(pdu),
            flags: ChannelFlags::empty(),
        }
    }
}

/// Defines which compression flag should be sent along the [`ChannelDef`] structure (CHANNEL_DEF)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionCondition {
    /// Virtual channel data will not be compressed
    Never,
    /// Virtual channel data MUST be compressed if RDP data is being compressed (CHANNEL_OPTION_COMPRESS_RDP)
    WhenRdpDataIsCompressed,
    /// Virtual channel data MUST be compressed, regardless of RDP compression settings (CHANNEL_OPTION_COMPRESS)
    Always,
}

/// A static virtual channel.
#[derive(Debug)]
pub struct StaticVirtualChannel {
    channel_processor: Box<dyn SvcProcessor>,
    chunk_processor: ChunkProcessor,
}

impl StaticVirtualChannel {
    pub fn new<T: SvcProcessor + 'static>(channel_processor: T) -> Self {
        Self {
            channel_processor: Box::new(channel_processor),
            chunk_processor: ChunkProcessor::new(),
        }
    }

    pub fn channel_name(&self) -> ChannelName {
        self.channel_processor.channel_name()
    }

    pub fn compression_condition(&self) -> CompressionCondition {
        self.channel_processor.compression_condition()
    }

    pub fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        self.channel_processor.start()
    }

    /// Processes a payload received on the virtual channel. Returns a vector of PDUs to be sent back
    /// to the server. If no PDUs are to be sent, an empty vector is returned.
    pub fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        if let Some(payload) = self.dechunkify(payload)? {
            return self.channel_processor.process(&payload);
        }

        Ok(Vec::new())
    }

    pub fn chunkify(messages: Vec<SvcMessage>) -> PduResult<Vec<WriteBuf>> {
        ChunkProcessor::chunkify(messages, CHANNEL_CHUNK_LENGTH)
    }

    pub fn channel_processor_downcast_ref<T: SvcProcessor + 'static>(&self) -> Option<&T> {
        self.channel_processor.as_any().downcast_ref()
    }

    pub fn channel_processor_downcast_mut<T: SvcProcessor + 'static>(&mut self) -> Option<&mut T> {
        self.channel_processor.as_any_mut().downcast_mut()
    }

    fn dechunkify(&mut self, payload: &[u8]) -> PduResult<Option<Vec<u8>>> {
        self.chunk_processor.dechunkify(payload)
    }
}

fn encode_svc_messages(
    messages: Vec<SvcMessage>,
    channel_id: u16,
    initiator_id: u16,
    client: bool,
) -> PduResult<Vec<u8>> {
    let mut fully_encoded_responses = WriteBuf::new(); // TODO(perf): reuse this buffer using `clear` and `filled` as appropriate

    // For each response PDU, chunkify it and add appropriate static channel headers.
    let chunks = StaticVirtualChannel::chunkify(messages)?;

    // SendData is [`McsPdu`], which is [`x224Pdu`], which is [`PduEncode`]. [`PduEncode`] for [`x224Pdu`]
    // also takes care of adding the Tpkt header, so therefore we can just call `encode_buf` on each of these and
    // we will create a buffer of fully encoded PDUs ready to send to the server.
    //
    // For example, if we had 2 chunks, our fully_encoded_responses buffer would look like:
    //
    // [ | tpkt | x224 | mcs::SendDataRequest | chunk 1 | tpkt | x224 | mcs::SendDataRequest | chunk 2 | ]
    //   |<------------------- PDU 1 ------------------>|<------------------- PDU 2 ------------------>|
    if client {
        for chunk in chunks {
            let pdu = mcs::SendDataRequest {
                initiator_id,
                channel_id,
                user_data: Cow::Borrowed(chunk.filled()),
            };
            encode_buf(&pdu, &mut fully_encoded_responses)?;
        }
    } else {
        for chunk in chunks {
            let pdu = mcs::SendDataIndication {
                initiator_id,
                channel_id,
                user_data: Cow::Borrowed(chunk.filled()),
            };
            encode_buf(&pdu, &mut fully_encoded_responses)?;
        }
    }

    Ok(fully_encoded_responses.into_inner())
}

/// Encode a vector of [`SvcMessage`] in preparation for sending them on the `channel_id` channel.
///
/// This includes chunkifying the messages, adding MCS, x224, and tpkt headers, and encoding them into a buffer.
/// The messages returned here are ready to be sent to the server.
///
/// The caller is responsible for ensuring that the `channel_id` corresponds to the correct channel.
pub fn client_encode_svc_messages(messages: Vec<SvcMessage>, channel_id: u16, initiator_id: u16) -> PduResult<Vec<u8>> {
    encode_svc_messages(messages, channel_id, initiator_id, true)
}

/// Encode a vector of [`SvcMessage`] in preparation for sending them on the `channel_id` channel.
///
/// This includes chunkifying the messages, adding MCS, x224, and tpkt headers, and encoding them into a buffer.
/// The messages returned here are ready to be sent to the client.
///
/// The caller is responsible for ensuring that the `channel_id` corresponds to the correct channel.
pub fn server_encode_svc_messages(messages: Vec<SvcMessage>, channel_id: u16, initiator_id: u16) -> PduResult<Vec<u8>> {
    encode_svc_messages(messages, channel_id, initiator_id, false)
}

/// A type that is a Static Virtual Channel
///
/// Static virtual channels are created once at the beginning of the RDP session and allow lossless
/// communication between client and server components over the main data connection.
/// There are at most 31 (optional) static virtual channels that can be created for a single connection, for a
/// total of 32 static channels when accounting for the non-optional I/O channel.
pub trait SvcProcessor: ironrdp_core::AsAny + fmt::Debug + Send {
    /// Returns the name of the static virtual channel corresponding to this processor.
    fn channel_name(&self) -> ChannelName;

    /// Defines which compression flag should be sent along the [`ChannelDef`] Definition Structure (`CHANNEL_DEF`)
    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    /// Start a channel, after the connection is established and the channel is joined.
    ///
    /// Returns a list of PDUs to be sent back.
    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    /// Processes a payload received on the virtual channel. The `payload` is expected
    /// to be a fully de-chunkified PDU.
    ///
    /// Returns a list of PDUs to be sent back.
    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>>;
}

assert_obj_safe!(SvcProcessor);

pub trait SvcClientProcessor: SvcProcessor {}

assert_obj_safe!(SvcClientProcessor);

pub trait SvcServerProcessor: SvcProcessor {}

assert_obj_safe!(SvcServerProcessor);

/// ChunkProcessor is used to chunkify/de-chunkify static virtual channel PDUs.
#[derive(Debug)]
struct ChunkProcessor {
    /// Buffer for de-chunkification of clipboard PDUs. Everything bigger than ~1600 bytes is
    /// usually chunked when transferred over svc.
    chunked_pdu: Vec<u8>,
}

impl ChunkProcessor {
    fn new() -> Self {
        Self {
            chunked_pdu: Vec::new(),
        }
    }

    /// Takes a vector of PDUs and breaks them into chunks prefixed with a Channel PDU Header (`CHANNEL_PDU_HEADER`).
    ///
    /// Each chunk is at most `max_chunk_len` bytes long (not including the Channel PDU Header).
    fn chunkify(messages: Vec<SvcMessage>, max_chunk_len: usize) -> PduResult<Vec<WriteBuf>> {
        let mut results = Vec::new();
        for message in messages {
            results.extend(Self::chunkify_one(message, max_chunk_len)?);
        }
        Ok(results)
    }

    /// Dechunkify a payload received on the virtual channel.
    ///
    /// If the payload is not chunked, returns the payload as-is.
    /// For chunked payloads, returns `Ok(None)` until the last chunk is received, at which point
    /// it returns `Ok(Some(payload))`.
    fn dechunkify(&mut self, payload: &[u8]) -> PduResult<Option<Vec<u8>>> {
        let mut cursor = ReadCursor::new(payload);
        let last = Self::process_header(&mut cursor)?;

        // Extend the chunked_pdu buffer with the payload
        self.chunked_pdu.extend_from_slice(cursor.remaining());

        // If this was an unchunked message, or the last in a series of chunks, return the payload
        if last {
            // Take the chunked_pdu buffer and replace it with an empty one
            return Ok(Some(std::mem::take(&mut self.chunked_pdu)));
        }

        // This was an intermediate chunk, return None
        Ok(None)
    }

    /// Returns whether this was the last chunk based on the flags in the channel header.
    fn process_header(payload: &mut ReadCursor<'_>) -> PduResult<bool> {
        let channel_header: ironrdp_pdu::rdp::vc::ChannelPduHeader = decode_cursor(payload)?;

        Ok(channel_header.flags.contains(ChannelControlFlags::FLAG_LAST))
    }

    /// Takes a single PDU and breaks it into chunks prefixed with a [`ChannelPduHeader`].
    ///
    /// Each chunk is at most `max_chunk_len` bytes long (not including the Channel PDU Header).
    ///
    /// For example, if the PDU is 4000 bytes long and `max_chunk_len` is 1600, this function will
    /// return 3 chunks, each 1600 bytes long, and the last chunk will be 800 bytes long.
    ///
    /// [[ Channel PDU Header | 1600 bytes of PDU data ] [ Channel PDU Header | 1600 bytes of PDU data ] [ Channel PDU Header | 800 bytes of PDU data ]]
    fn chunkify_one(message: SvcMessage, max_chunk_len: usize) -> PduResult<Vec<WriteBuf>> {
        let mut encoded_pdu = WriteBuf::new(); // TODO(perf): reuse this buffer using `clear` and `filled` as appropriate
        encode_buf(message.pdu.as_ref(), &mut encoded_pdu)?;

        let mut chunks = Vec::new();

        let total_len = encoded_pdu.filled_len();
        let mut chunk_start_index: usize = 0;
        let mut chunk_end_index = std::cmp::min(total_len, max_chunk_len);
        loop {
            // Create a buffer to hold this next chunk.
            // TODO(perf): Reuse this buffer using `clear` and `filled` as appropriate.
            //             This one will be a bit trickier because we'll need to grow
            //             the number of chunk buffers if we run out.
            let mut chunk = WriteBuf::new();

            // Set the first and last flags if this is the first and/or last chunk for this PDU.
            let first = chunk_start_index == 0;
            let last = chunk_end_index == total_len;

            // Create the header for this chunk.
            let header = {
                let mut flags = ChannelFlags::empty();
                if first {
                    flags |= ChannelFlags::FIRST;
                }
                if last {
                    flags |= ChannelFlags::LAST;
                }

                flags |= message.flags;

                ChannelPduHeader {
                    length: ironrdp_pdu::cast_int!(ChannelPduHeader::NAME, "length", total_len)?,
                    flags,
                }
            };

            // Encode the header for this chunk.
            encode_buf(&header, &mut chunk)?;
            // Append the piece of the encoded_pdu that belongs in this chunk.
            chunk.write_slice(&encoded_pdu[chunk_start_index..chunk_end_index]);
            // Push the chunk onto the results.
            chunks.push(chunk);

            // If this was the last chunk, we're done, return the results.
            if last {
                break;
            }

            // Otherwise, update the chunk start and end indices for the next iteration.
            chunk_start_index = chunk_end_index;
            chunk_end_index = std::cmp::min(total_len, chunk_end_index.saturating_add(max_chunk_len));
        }

        Ok(chunks)
    }
}

impl Default for ChunkProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the [`ChannelOptions`] bitfield to be used in the [`ChannelDef`] structure.
pub fn make_channel_options(channel: &StaticVirtualChannel) -> ChannelOptions {
    match channel.compression_condition() {
        CompressionCondition::Never => ChannelOptions::empty(),
        CompressionCondition::WhenRdpDataIsCompressed => ChannelOptions::COMPRESS_RDP,
        CompressionCondition::Always => ChannelOptions::COMPRESS,
    }
}

/// Builds the [`ChannelDef`] structure containing information for this channel.
pub fn make_channel_definition(channel: &StaticVirtualChannel) -> ChannelDef {
    let name = channel.channel_name();
    let options = make_channel_options(channel);
    ChannelDef { name, options }
}

/// A set holding at most one [`StaticVirtualChannel`] for any given type
/// implementing [`SvcProcessor`].
///
/// To ensure uniqueness, each trait object is associated to the [`TypeId`] of it’s original type.
/// Once joined, channels may have their ID attached using [`Self::attach_channel_id()`], effectively
/// associating them together.
///
/// At this point, it’s possible to retrieve the trait object using either
/// the type ID ([`Self::get_by_type_id()`]), the original type ([`Self::get_by_type()`]) or
/// the channel ID ([`Self::get_by_channel_id()`]).
///
/// It’s possible to downcast the trait object and to retrieve the concrete value
/// since all [`SvcProcessor`]s are also implementing the [`AsAny`] trait.
#[derive(Debug)]
pub struct StaticChannelSet {
    channels: BTreeMap<TypeId, StaticVirtualChannel>,
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

    /// Inserts a [`StaticVirtualChannel`] into this [`StaticChannelSet`].
    ///
    /// If a static virtual channel of this type already exists, it is returned.
    pub fn insert<T: SvcProcessor + 'static>(&mut self, val: T) -> Option<StaticVirtualChannel> {
        self.channels.insert(TypeId::of::<T>(), StaticVirtualChannel::new(val))
    }

    /// Gets a reference to a [`StaticVirtualChannel`] by looking up its internal [`SvcProcessor`]'s [`TypeId`].
    pub fn get_by_type_id(&self, type_id: TypeId) -> Option<&StaticVirtualChannel> {
        self.channels.get(&type_id)
    }

    /// Gets a mutable reference to a [`StaticVirtualChannel`] by looking up its internal [`SvcProcessor`]'s [`TypeId`].
    pub fn get_by_type_id_mut(&mut self, type_id: TypeId) -> Option<&mut StaticVirtualChannel> {
        self.channels.get_mut(&type_id)
    }

    /// Gets a reference to a [`StaticVirtualChannel`] by looking up its internal [`SvcProcessor`]'s [`TypeId`].
    pub fn get_by_type<T: SvcProcessor + 'static>(&self) -> Option<&StaticVirtualChannel> {
        self.get_by_type_id(TypeId::of::<T>())
    }

    /// Gets a mutable reference to a [`StaticVirtualChannel`] by looking up its internal [`SvcProcessor`]'s [`TypeId`].
    pub fn get_by_type_mut<T: SvcProcessor + 'static>(&mut self) -> Option<&mut StaticVirtualChannel> {
        self.get_by_type_id_mut(TypeId::of::<T>())
    }

    /// Gets a reference to a [`StaticVirtualChannel`] by looking up its channel name.
    pub fn get_by_channel_name(&self, name: &ChannelName) -> Option<(TypeId, &StaticVirtualChannel)> {
        self.iter().find(|(_, x)| x.channel_processor.channel_name() == *name)
    }

    /// Gets a reference to a [`StaticVirtualChannel`] by looking up its channel ID.
    pub fn get_by_channel_id(&self, channel_id: StaticChannelId) -> Option<&StaticVirtualChannel> {
        self.get_type_id_by_channel_id(channel_id)
            .and_then(|type_id| self.get_by_type_id(type_id))
    }

    /// Gets a mutable reference to a [`StaticVirtualChannel`] by looking up its channel ID.
    pub fn get_by_channel_id_mut(&mut self, channel_id: StaticChannelId) -> Option<&mut StaticVirtualChannel> {
        self.get_type_id_by_channel_id(channel_id)
            .and_then(|type_id| self.get_by_type_id_mut(type_id))
    }

    /// Removes a [`StaticVirtualChannel`] from this [`StaticChannelSet`].
    ///
    /// If a static virtual channel of this type existed, it will be returned.
    pub fn remove_by_type_id(&mut self, type_id: TypeId) -> Option<StaticVirtualChannel> {
        let svc = self.channels.remove(&type_id);
        if let Some(channel_id) = self.to_channel_id.remove(&type_id) {
            self.to_type_id.remove(&channel_id);
        }
        svc
    }

    /// Removes a [`StaticVirtualChannel`] from this [`StaticChannelSet`].
    ///
    /// If a static virtual channel of this type existed, it will be returned.
    pub fn remove_by_type<T: SvcProcessor + 'static>(&mut self) -> Option<StaticVirtualChannel> {
        let type_id = TypeId::of::<T>();
        self.remove_by_type_id(type_id)
    }

    /// Attaches a channel ID to a static virtual channel.
    ///
    /// If a channel ID was already attached, it will be returned.
    pub fn attach_channel_id(&mut self, type_id: TypeId, channel_id: StaticChannelId) -> Option<StaticChannelId> {
        self.to_type_id.insert(channel_id, type_id);
        self.to_channel_id.insert(type_id, channel_id)
    }

    /// Gets the attached channel ID for a given static virtual channel.
    pub fn get_channel_id_by_type_id(&self, type_id: TypeId) -> Option<StaticChannelId> {
        self.to_channel_id.get(&type_id).copied()
    }

    /// Gets the attached channel ID for a given static virtual channel.
    pub fn get_channel_id_by_type<T: SvcProcessor + 'static>(&self) -> Option<StaticChannelId> {
        self.get_channel_id_by_type_id(TypeId::of::<T>())
    }

    /// Gets the [`TypeId`] of the static virtual channel associated to this channel ID.
    pub fn get_type_id_by_channel_id(&self, channel_id: StaticChannelId) -> Option<TypeId> {
        self.to_type_id.get(&channel_id).copied()
    }

    /// Detaches the channel ID associated to a given static virtual channel.
    pub fn detach_channel_id(&mut self, type_id: TypeId) -> Option<StaticChannelId> {
        if let Some(channel_id) = self.to_channel_id.remove(&type_id) {
            self.to_type_id.remove(&channel_id);
            Some(channel_id)
        } else {
            None
        }
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (TypeId, &StaticVirtualChannel)> {
        self.channels.iter().map(|(type_id, svc)| (*type_id, svc))
    }

    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (TypeId, &mut StaticVirtualChannel, Option<StaticChannelId>)> {
        let to_channel_id = self.to_channel_id.clone();
        self.channels
            .iter_mut()
            .map(move |(type_id, svc)| (*type_id, svc, to_channel_id.get(type_id).copied()))
    }

    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &StaticVirtualChannel> {
        self.channels.values()
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
/// - <https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/6c074267-1b32-4ceb-9496-2eb941a23e6b>
/// - <https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/a8593178-80c0-4b80-876c-cb77e62cecfc>
pub const CHANNEL_CHUNK_LENGTH: usize = 1600;

bitflags! {
    /// Channel control flags, as specified in [section 2.2.6.1.1 of MS-RDPBCGR].
    ///
    /// [section 2.2.6.1.1 of MS-RDPBCGR]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/f125c65e-6901-43c3-8071-d7d5aaee7ae4
    #[derive(Debug, PartialEq, Copy, Clone)]
    pub struct ChannelFlags: u32 {
        /// CHANNEL_FLAG_FIRST
        const FIRST = 0x0000_0001;
        /// CHANNEL_FLAG_LAST
        const LAST = 0x0000_0002;
        /// CHANNEL_FLAG_SHOW_PROTOCOL
        const SHOW_PROTOCOL = 0x0000_0010;
        /// CHANNEL_FLAG_SUSPEND
        const SUSPEND = 0x0000_0020;
        /// CHANNEL_FLAG_RESUME
        const RESUME = 0x0000_0040;
        /// CHANNEL_FLAG_SHADOW_PERSISTENT
        const SHADOW_PERSISTENT = 0x0000_0080;
        /// CHANNEL_PACKET_COMPRESSED
        const COMPRESSED = 0x0020_0000;
        /// CHANNEL_PACKET_AT_FRONT
        const AT_FRONT = 0x0040_0000;
        /// CHANNEL_PACKET_FLUSHED
        const FLUSHED = 0x0080_0000;
    }
}

/// Channel PDU Header (CHANNEL_PDU_HEADER)
///
/// Channel PDU header precedes all static virtual channel traffic
/// transmitted between an RDP client and server.
///
/// It is specified in [section 2.2.6.1.1 of MS-RDPBCGR].
///
/// [section 2.2.6.1.1 of MS-RDPBCGR]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/f125c65e-6901-43c3-8071-d7d5aaee7ae4
#[derive(Debug)]
struct ChannelPduHeader {
    /// The total length of the uncompressed PDU data,
    /// excluding the length of this header.
    /// Note: the data can span multiple PDUs, in which
    /// case each PDU in the series contains the same
    /// length field.
    length: u32,
    flags: ChannelFlags,
}

impl ChannelPduHeader {
    const NAME: &'static str = "CHANNEL_PDU_HEADER";

    const FIXED_PART_SIZE: usize = 4 /* len */ + 4 /* flags */;
}

impl PduEncode for ChannelPduHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u32(self.length);
        dst.write_u32(self.flags.bits());
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    #[allow(clippy::arithmetic_side_effects)]
    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}
