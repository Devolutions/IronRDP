#![allow(clippy::unnecessary_box_returns)] // Diplomat requires returning Boxed types
pub mod connector;
pub mod credssp;
pub mod dvc;
pub mod error;
pub mod ironrdp_blocking;
pub mod svc;
pub mod tls;
pub mod utils;

use sspi as _; // we need this for network_client and avoid CI failure
use tracing as _; // need this in the future
