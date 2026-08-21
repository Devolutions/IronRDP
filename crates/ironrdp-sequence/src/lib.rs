#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "state-machine")]
mod macros;

/// Not part of the public API. Exposed only so `reason_err!`'s expansion can
/// reach `format!` through a `$crate`-qualified path.
///
/// A bare `format!` written inside a `macro_rules!` body is not fully
/// hygienic: unlike item and type names, an unqualified macro invocation
/// falls back to the *caller's* scope when it is not found at the
/// definition site. A caller that has not itself imported `alloc::format`
/// (e.g. a `no_std` crate depending on the `state-machine` feature) would
/// then fail to compile `reason_err!(...)`, regardless of whether this
/// crate itself is built with `alloc`/`std`. Routing through `$crate`
/// resolves unconditionally against this crate's own `alloc`, sidestepping
/// the caller's scope entirely. See `tests/reason_err.rs` for the
/// regression test.
#[cfg(feature = "state-machine")]
#[doc(hidden)]
pub mod __private {
    pub use alloc::format;
}

mod desktop_size;
#[cfg(feature = "state-machine")]
mod negotiation_failure;
#[cfg(feature = "state-machine")]
mod sequence;
#[cfg(feature = "state-machine")]
mod sequence_error;
#[cfg(feature = "alloc")]
mod server_name;
#[cfg(feature = "state-machine")]
mod state;
#[cfg(feature = "state-machine")]
mod step_input;
mod time;
#[cfg(feature = "state-machine")]
mod written;

pub use self::desktop_size::DesktopSize;
#[cfg(feature = "state-machine")]
pub use self::negotiation_failure::NegotiationFailure;
#[cfg(feature = "state-machine")]
pub use self::sequence::Sequence;
#[cfg(feature = "state-machine")]
pub use self::sequence_error::{SequenceError, SequenceErrorExt, SequenceErrorKind, SequenceResult, SequenceResultExt};
#[cfg(feature = "alloc")]
pub use self::server_name::ServerName;
#[cfg(feature = "state-machine")]
pub use self::state::{State, state_downcast, state_is};
#[cfg(feature = "state-machine")]
pub use self::step_input::StepInput;
pub use self::time::MonotonicInstant;
#[cfg(feature = "state-machine")]
pub use self::written::Written;
