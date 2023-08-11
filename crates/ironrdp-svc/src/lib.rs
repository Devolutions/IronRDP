#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;

use alloc::boxed::Box;
use core::fmt;

use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{assert_obj_safe, PduResult};
use pdu::gcc::Channel;

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
pub trait StaticVirtualChannel: fmt::Debug + Send + Sync {
    #[doc(hidden)]
    fn is_drdynvc(&self) -> bool {
        // FIXME: temporary method that will be removed once drdynvc is ported to the new API
        false
    }

    /// Returns the name of the `StaticVirtualChannel`
    fn channel_name(&self) -> ChannelName;

    /// Defines which compression flag should be sent along the Channel Definition Structure (CHANNEL_DEF)
    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    /// Processes a complete block (chunks must be assembled by calling code)
    fn process(&mut self, initiator_id: u16, channel_id: u16, payload: &[u8], output: &mut WriteBuf) -> PduResult<()>;

    /// Build the Channel Definition Structure (CHANNEL_DEF) containing information for this channel.
    fn make_channel_definition(&self) -> Channel {
        let name = self.channel_name();
        let options = match self.compression_condition() {
            CompressionCondition::Never => ChannelOptions::empty(),
            CompressionCondition::WhenRdpDataIsCompressed => ChannelOptions::COMPRESS_RDP,
            CompressionCondition::Always => ChannelOptions::COMPRESS,
        };
        Channel { name, options }
    }
}

assert_obj_safe!(StaticVirtualChannel);
