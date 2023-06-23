#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::any::{Any, TypeId};
use core::fmt;

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
    /// Returns the name of the `StaticVirtualChannel` that is created by the `make_static_channel` method
    fn channel_name(&self) -> ChannelName;

    /// Defines which compression flag should be sent along the Channel Definition Structure (CHANNEL_DEF)
    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    /// Processes a complete block (chunks must be assembled by calling code)
    fn process(
        &mut self,
        initiator_id: StaticChannelId,
        channel_id: StaticChannelId,
        payload: &[u8],
        output: &mut WriteBuf,
    ) -> PduResult<()>;

    #[doc(hidden)]
    fn is_drdynvc(&self) -> bool {
        // FIXME: temporary method that will be removed once drdynvc is ported to the new API
        false
    }
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
