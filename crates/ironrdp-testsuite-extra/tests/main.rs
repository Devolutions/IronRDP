#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

mod client_config;
mod e2e;