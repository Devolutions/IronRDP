#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

mod buffer;
mod process;
#[cfg(target_arch = "wasm32")]
mod replay;

/// Direction/source of a PDU in the recording.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PduSource {
    /// A PDU from the client (C→S).
    Client = 0,
    /// A PDU from the server (S→C).
    Server = 1,
}

pub use buffer::PduBuffer;
pub use process::{
    PointerState, ProcessResult, ProcessTillResult, ReplayError, ReplayErrorKind, ReplayProcessor,
    ReplayProcessorConfig, ReplayResult, UpdateKind,
};
#[cfg(target_arch = "wasm32")]
pub use replay::Replay;
