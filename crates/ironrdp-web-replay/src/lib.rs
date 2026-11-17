use wasm_bindgen::prelude::*;

mod buffer;
mod process;
mod replay;

/// Direction/source of a PDU in the recording.
#[wasm_bindgen]
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
pub use replay::Replay;
