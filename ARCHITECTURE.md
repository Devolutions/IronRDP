# Architecture

This document describes the high-level architecture of IronRDP.

> Roughly, it takes 2x more time to write a patch if you are unfamiliar with the
> project, but it takes 10x more time to figure out where you should change the
> code.

[Source](https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html)

## Code Map

This section talks briefly about various important directories and data structures.

### Core Tier

Set of foundational libraries for which strict quality standards must be observed.
Pay attention to the "**Architecture Invariant**" sections.

- `crates/ironrdp`: meta crate re-exporting important crates.
- `crates/ironrdp-pdu`: PDU encoding and decoding. (TODO: talk about important types and traits such as PduDecode, PduEncode…)
- `crates/ironrdp-graphics`: image processing primitives.
- `crates/ironrdp-connector`: state machines to drive an RDP connection sequence.
- `crates/ironrdp-session`: state machines to drive an RDP session.
- `crates/ironrdp-input`: utilities to manage and build input packets.
- `crates/ironrdp-rdcleanpath`: RDCleanPath PDU structure used by IronRDP web client and Devolutions Gateway.
- `crates/ironrdp-error`: lightweight and `no_std`-compatible generic `Error` and `Report` types.
  The `Error` type wraps a custom consumer-defined type for domain-specific details (such as `PduErrorKind`).

**Architectural Invariant**: doing I/O is not allowed for these crates.

**Architectural Invariant**: all these crates must be fuzzed.

**Architectural Invariant**: no non-essential dependency is allowed.

**Architectural Invariant**: must be `#[no_std]`-compatible (optionally using the `alloc` crate). Usage of the standard
library must be opt-in through a feature flag called `std` that is enabled by default. When the `alloc` crate is optional,
a feature flag called `alloc` must exist to enable its use.

### Extra Tier

Higher level libraries and binaries built on top of the core tier.
Guidelines and constraints are relaxed to some extent.

- `crates/ironrdp-async`: provides `Future`s wrapping the state machines conveniently.
- `crates/ironrdp-tokio`: `Framed*` traits implementation above `tokio`’s traits.
- `crates/ironrdp-futures`: `Framed*` traits implementation above `futures`’s traits.
- `crates/ironrdp-tls`: TLS boilerplate common with most IronRDP clients.
- `crates/ironrdp-client`: portable RDP client without GPU acceleration using softbuffer and winit for windowing.
- `crates/ironrdp-web`: WebAssembly high-level bindings targeting web browsers.
- `web-client/iron-remote-gui`: core frontend UI used by `iron-svelte-client` as a Web Component.
- `web-client/iron-svelte-client`: web-based frontend using `Svelte` and `Material` frameworks.

### Internal Tier

Crates that are only used inside the IronRDP project, not meant to be published.
This is mostly test case generators, fuzzing oracles, build tools, and so on.

- `crates/ironrdp-pdu-generators`: `proptest` generators for `ironrdp-pdu` types.
- `crates/ironrdp-session-generators`: `proptest` generators for `ironrdp-session` types.
- `crates/ironrdp-testsuite-core`: contains all integration tests for code living in the core tier, in a single binary,
  organized in modules. **Architectural Invariant**: no dependency from another tier is allowed.
  It must be the case that compiling and running the core test suite does not require building any library from
  the extra tier. This is to keep iteration time short.
- `crates/ironrdp-testsuite-extra`: contains all integration tests for code living in the extra tier, in a single binary,
  organized in modules. (WIP: this crate does not exist yet.)
- `crates/ironrdp-fuzzing`: provides test case generators and oracles for use with fuzzing.
- `fuzz`: fuzz targets for code in core tier.
- `xtask`: IronRDP’s free-form automation using Rust code.

**Architecture Invariant**: these crates are not, and will never be, an **API Boundary**.

### Community Tier

Crates provided and maintained by the community.
Core maintainers will not invest a lot of time into these.
One or several community maintainers are associated to each one 

- `crates/ironrdp-glutin-renderer` (no maintainer): `glutin` primitives for OpenGL rendering.
- `crates/ironrdp-client-glutin` (no maintainer): GPU-accelerated RDP client using glutin.
- `crates/ironrdp-replay-client` (no maintainer): utility tool to replay RDP graphics pipeline for debugging purposes.

## Cross-Cutting Concerns

This section talks about the things which are everywhere and nowhere in particular.

### General

- Dependency injection when runtime information is necessary in core tier crates (no system call such as `gethostname`)
- Keep non-portable code out of core tier crates
- Make crate `no_std`-compatible wherever possible
- Facilitate fuzzing
- In libraries, provide concrete error types either hand-crafted or using `thiserror` crate
- In binaries, use the convenient catch-all error type `anyhow::Error`
- Free-form automation a-la `make` following [`cargo xtask`](https://github.com/matklad/cargo-xtask) specification

### Avoid I/O wherever possible

**Architecture Invariant**: core tier crates must never interact with the outside world. Only extra tier crates
such as `ironrdp-client`, `ironrdp-web` or `ironrdp-async` are allowed to do I/O.

### Continuous integration

We use GitHub action and our workflows simply run `cargo xtask`.
The expectation is that, if `cargo xtask ci` passes locally, the CI will be green as well.

**Architecture Invariant**: `cargo xtask ci` and CI workflow must be logically equivalents. It must
be the case that a successful `cargo xtask ci` run implies a successful CI workflow run and vice versa.

### Testing

#### Test at the boundaries (test features, not code)

We should focus on testing the public API of libraries (keyword: **API boundary**).
That’s why most (if not all) tests should go into the `ironrdp-testsuite-core` and `ironrdp-testsuite-extra` crates.

#### Do not depend on external resources

**Architecture Invariant**: tests do not depend on any kind of external resources, they are perfectly reproducible.

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
