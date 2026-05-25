// Enable WebAssembly-compatible RNG backends for transitive dependencies
// (picky → crypto-bigint → getrandom). These are not used directly.
extern crate getrandom as _;
extern crate getrandom2 as _;
extern crate getrandom4 as _;

use wasm_bindgen::prelude::*;

mod buffer;
mod process;
mod replay;

/// Direction/source of a PDU in the recording.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PduSource {
    /// PDU from client (C→S)
    Client = 0,
    /// PDU from server (S→C)
    Server = 1,
}

pub use buffer::PduBuffer;
pub use process::{
    PointerState, ProcessResult, ProcessTillResult, ReplayError, ReplayErrorExt, ReplayErrorKind, ReplayProcessor,
    ReplayProcessorConfig, ReplayResult, UpdateKind,
};
pub use replay::Replay;
