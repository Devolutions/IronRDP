#![allow(clippy::unnecessary_box_returns)] // Diplomat requires returning Boxed types
#![allow(clippy::should_implement_trait)] // Implementing extra traits is not useful for FFI
#![allow(clippy::needless_lifetimes)] // Diplomat requires lifetimes to be specified even if they can be elided in rugular Rust code

pub mod clipboard;
pub mod connector;
pub mod credssp;
pub mod dvc;
pub mod error;
pub mod graphics;
pub mod input;
pub mod log;
pub mod pdu;
pub mod session;
pub mod svc;
pub mod utils;
