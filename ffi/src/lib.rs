#![allow(clippy::unnecessary_box_returns)] // Diplomat requires returning Boxed types
#![allow(clippy::should_implement_trait)] // Implementing extra traits is not useful for FFI

pub mod connector;
pub mod credssp;
pub mod dvc;
pub mod error;
pub mod graphics;
pub mod input;
pub mod pdu;
pub mod session;
pub mod svc;
pub mod utils;
