//! `reason_err!` calls `format!` in its expansion. Rust resolves a bare macro
//! invocation written inside a `macro_rules!` body against the *caller's*
//! scope when it is not otherwise in scope at the definition site (unlike
//! item/type names, which are fully hygienic) -- so a caller that does not
//! have `format!` in scope (a no_std crate that has not imported
//! `alloc::format`) fails to compile `reason_err!(...)`, no matter what
//! features `ironrdp-sequence` itself is built with.
//!
//! `#![no_implicit_prelude]` reproduces that caller shape cheaply, without a
//! full `#![no_std]` binary: it strips the std prelude -- including
//! `format!` -- from this file's scope while keeping `std` itself linked, so
//! `#[test]` still works. This only proves something if `reason_err!`
//! resolves `format!` through `$crate`, not a bare call: see
//! `crate::__private` in `ironrdp-sequence`'s `src/lib.rs`.
#![no_implicit_prelude]

use ::ironrdp_sequence::reason_err;

// These are dependencies of `ironrdp-sequence` itself, linked into every one of
// its test binaries regardless of whether this particular test exercises them.
use ::ironrdp_core as _;
use ::ironrdp_error as _;
use ::ironrdp_pdu as _;

#[test]
fn reason_err_expands_without_format_in_caller_scope() {
    let _err = reason_err!("test context", "value {}", 42);
}
