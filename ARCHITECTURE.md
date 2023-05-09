# Architecture

This document describes the high-level architecture of IronRDP.

> Roughly, it takes 2x more time to write a patch if you are unfamiliar with the
> project, but it takes 10x more time to figure out where you should change the
> code.

[Source](https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html)

## Code Map

This section talks briefly about various important directories and data structures.

### Core Crates

- `crates/ironrdp`: meta crate re-exporting important crates.
- `crates/ironrdp-pdu`: PDU encoding and decoding (no I/O, trivial to fuzz). <!-- TODO: important types and traits (PduDecode, PduEncode…) -->
- `crates/ironrdp-graphics`: image processing primitives (no I/O, trivial to fuzz).
- `crates/ironrdp-connector`: state machines to drive an RDP connection sequence (no I/O, not _too_ hard to fuzz).
- `crates/ironrdp-session`: state machines to drive an RDP session (no I/O, not _too_ hard to fuzz).
- `crates/ironrdp-input`: utilities to manage and build input packets (no I/O).
- `crates/ironrdp-rdcleanpath`: RDCleanPath PDU structure used by IronRDP web client and Devolutions Gateway.

### Utility Crates

- `crates/ironrdp-async`: provides `Future`s wrapping the state machines conveniently.
- `crates/ironrdp-tokio`: `Framed*` traits implementation above `tokio`’s traits.
- `crates/ironrdp-futures`: `Framed*` traits implementation above `futures`’s traits.
- `crates/ironrdp-tls`: TLS boilerplate common with most IronRDP clients.

### Client Crates

- `crates/ironrdp-client`: portable RDP client without GPU acceleration using softbuffer and winit for windowing.
- `crates/ironrdp-web`: WebAssembly high-level bindings targeting web browsers.
- `crates/ironrdp-glutin-renderer`: `glutin` primitives for OpenGL rendering.
- `crates/ironrdp-client-glutin`: GPU-accelerated RDP client using glutin.
- `crates/ironrdp-replay-client`: utility tool to replay RDP graphics pipeline for debugging purposes.
- `web-client/iron-remote-gui`: core frontend UI used by `iron-svelte-client` as a Web Component.
- `web-client/iron-svelte-client`: web-based frontend using `Svelte` and `Material` frameworks.

### Private Crates

Crates that are only used inside the IronRDP project, not meant to be published.

- `crates/ironrdp-pdu-generators`: `proptest` generators for `ironrdp-pdu` types.
- `crates/ironrdp-session-generators`: `proptest` generators for `ironrdp-session` types.
- `fuzz`: fuzz targets for core crates.
- `xtask`: IronRDP’s free-form automation using Rust code.

## Cross-Cutting Concerns

This section talks about the things which are everywhere and nowhere in particular.

### General

- Dependency injection when runtime information is necessary in core crates (no system call such as `gethostname`)
- Keep non-portable code out of core crates
- Make crate `no_std`-compatible wherever possible
- Facilitate fuzzing
- In libraries, provide concrete error types either hand-crafted or using `thiserror` crate
- In binaries, use the convenient catch-all error type `anyhow::Error`
- Free-form automation a-la `make` following [`cargo xtask`](https://github.com/matklad/cargo-xtask) specification

### Avoid I/O wherever possible

**Architecture Invariant**: core crates must never interact with the outside world. Only client and utility crates
such as `ironrdp-client`, `ironrdp-web` or `ironrdp-async` are allowed to do I/O.

### Continuous integration

We use GitHub action and our workflows simply run `cargo xtask`.
The expectation is that, if `cargo xtask ci` passes locally, the CI will be green as well.

**Architecture Invariant**: `cargo xtask ci` and CI workflow must be logically equivalents. It must
be the case that a successful `cargo xtask ci` run implies a successful CI workflow run and vice versa.

### Testing

#### Fuzzing

See [`fuzz/README.md`](../fuzz/README.md).

#### Readability

Do not include huge binary chunks directly in source files (`*.rs`). Place these in separate files (`*.bin`, `*.bmp`)
and include them using macros such as `include_bytes!` or `include_str!`.

#### Use `expect-test` for snapshot testing

When comparing structured data (e.g.: error results, decoded PDUs), use `expect-test`. It is both easy to create
and maintain such tests. When something affecting the representation is changed, simply run the test again with
`UPDATE_EXPECT=1` env variable to magically update the code.

See:
- <https://matklad.github.io/2021/05/31/how-to-test.html#Expect-Tests>
- <https://docs.rs/expect-test/latest/expect_test/>

TODO: take further inspiration from rust-analyzer
- https://github.com/rust-lang/rust-analyzer/blob/d7c99931d05e3723d878bea5dc26766791fa4e69/docs/dev/architecture.md#testing
- https://matklad.github.io/2021/05/31/how-to-test.html

#### Use `rstest` for fixture-based testing

When a test can be generalized for multiple inputs, use [`rstest`](https://github.com/la10736/rstest) to avoid code duplication.

#### Use `proptest` for property testing

It allows to test that certain properties of your code hold for arbitrary inputs, and if a failure
is found, automatically finds the minimal test case to reproduce the problem.
