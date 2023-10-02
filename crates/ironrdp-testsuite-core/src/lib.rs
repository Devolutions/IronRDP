// No need to be as strict as in production libraries
#![allow(clippy::arithmetic_side_effects)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

#[macro_use]
extern crate array_concat;
#[macro_use]
extern crate lazy_static;

#[macro_use]
mod macros;

pub mod capsets;
pub mod client_info;
pub mod cluster_data;
pub mod conference_create;
pub mod core_data;
pub mod gcc;
pub mod gfx;
pub mod graphics_messages;
pub mod mcs;
pub mod message_channel_data;
pub mod monitor_data;
pub mod monitor_extended_data;
pub mod multi_transport_channel_data;
pub mod network_data;
pub mod rdp;
pub mod security_data;

#[doc(hidden)]
pub use paste::paste;
