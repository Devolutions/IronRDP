#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "state-machine")]
mod macros;

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
pub use self::time::MonotonicInstant;
#[cfg(feature = "state-machine")]
pub use self::written::Written;
