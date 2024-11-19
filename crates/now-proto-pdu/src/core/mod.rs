//! This module contains `NOW-PROTO` core types definitions.

mod buffer;
mod header;
mod number;
mod status;
mod string;

pub use buffer::{NowLrgBuf, NowVarBuf};
pub use header::{NowHeader, NowMessageClass};
pub use number::{VarI16, VarI32, VarI64, VarU16, VarU32, VarU64};
pub use status::{NowSeverity, NowStatus, NowStatusCode};
pub use string::{NowLrgStr, NowString128, NowString16, NowString256, NowString32, NowString64, NowVarStr};
