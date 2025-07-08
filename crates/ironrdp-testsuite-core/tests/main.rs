#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

//! Integration Tests (IT)
//!
//! Integration tests are all contained in this single crate, and organized in modules.
//! This is to prevent `rustc` to re-link the library crates with each of the integration
//! tests (one for each *.rs file / test crate under the `tests/` folder).
//! Performance implication: https://github.com/rust-lang/cargo/pull/5022#issuecomment-364691154
//!
//! This is also good for execution performance.
//! Cargo will run all tests from a single binary in parallel, but
//! binaries themselves are run sequentially.

mod clipboard;
mod displaycontrol;
mod dvc;
mod fuzz_regression;
mod graphics;
mod input;
mod pcb;
mod pdu;
mod propertyset;
mod rdcleanpath;
mod rdpsnd;
mod server;
mod server_name;
mod session;
