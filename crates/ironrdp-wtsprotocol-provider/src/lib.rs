#![cfg(windows)]

mod auth_bridge;
mod connection;
mod listener;
mod manager;
mod session_registry;
mod wts_com;

pub use auth_bridge::{CredsspPolicy, CredsspServerBridge};
pub use connection::{ConnectionLifecycleState, ProtocolConnection};
pub use listener::ProtocolListener;
pub use manager::ProtocolManager;
pub use session_registry::{SessionEntry, SessionRegistry};
pub use wts_com::{create_protocol_manager_com, IRONRDP_PROTOCOL_MANAGER_CLSID, IRONRDP_PROTOCOL_MANAGER_CLSID_STR};
