#[macro_use]
extern crate log;

pub mod active_session;
pub mod connection_sequence;
pub mod frame;
pub mod image;
pub mod utils;

mod errors;
mod framed;

use ironrdp_core::gcc;

pub use crate::active_session::{ActiveStageOutput, ActiveStageProcessor, GfxHandler};
pub use crate::errors::RdpError;
pub use crate::framed::{ErasedWriter, FramedReader};

#[derive(Debug, Clone)]
pub struct GraphicsConfig {
    pub avc444: bool,
    pub h264: bool,
    pub thin_client: bool,
    pub small_cache: bool,
    pub capabilities: u32,
}

#[derive(Debug, Clone)]
pub struct InputConfig {
    pub credentials: sspi::AuthIdentity,
    pub security_protocol: ironrdp_core::SecurityProtocol,
    pub keyboard_type: gcc::KeyboardType,
    pub keyboard_subtype: u32,
    pub keyboard_functional_keys_count: u32,
    pub ime_file_name: String,
    pub dig_product_id: String,
    pub width: u16,
    pub height: u16,
    pub global_channel_name: String, // FIXME: <- should be removed (no stringified name actually exist in the spec)
    pub user_channel_name: String,   // FIXME: <- should be removed (no stringified name actually exist in the spec)
    pub graphics_config: Option<GraphicsConfig>,
}

#[derive(Copy, Clone, Debug)]
pub struct ChannelIdentificators {
    pub initiator_id: u16,
    pub channel_id: u16,
}
