# Architecture

This document describes the high-level architecture of IronRDP.

> Roughly, it takes 2x more time to write a patch if you are unfamiliar with the
> project, but it takes 10x more time to figure out where you should change the
> code.

[Source](https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html)

## Code Map

This section talks briefly about various important directories and data structures.

Note also which crates are **API Boundaries**.
Remember, [rules at the boundary are different](https://www.tedinski.com/2018/02/06/system-boundaries.html).

### Core Tier

Set of foundational libraries for which strict quality standards must be observed.
Note that all crates in this tier are **API Boundaries**.
Pay attention to the "**Architecture Invariant**" sections.

**Architectural Invariant**: doing I/O is not allowed for these crates.

**Architectural Invariant**: all these crates must be fuzzed.

**Architectural Invariant**: must be `#[no_std]`-compatible (optionally using the `alloc` crate). Usage of the standard
library must be opt-in through a feature flag called `std` that is enabled by default. When the `alloc` crate is optional,
a feature flag called `alloc` must exist to enable its use.

**Architectural Invariant**: no platform-dependant code (`#[cfg(windows)]` and such).

**Architectural Invariant**: no non-essential dependency is allowed.

**Architectural Invariant**: no proc-macro dependency. Dependencies such as `syn` should be pushed
as far as possible from the foundational crates so it doesn’t become too much of a compilation
bottleneck. [Compilation time is a multiplier for everything][why-care-about-build-time].
The paper [Developer Productivity For Humans, Part 4: Build Latency, Predictability,
and Developer Productivity][developer-productivity] by Ciera Jaspan and Collin Green, Google
researchers, also elaborates on why it is important to keep build times low.

**Architectural Invariant**: unless the performance, usability or ergonomic gain is really worth
it, the amount of [monomorphization] incurred in downstream user code should be minimal to avoid
binary bloating and to keep the compilation as parallel as possible. Large generic functions should
be avoided if possible.

[why-care-about-build-time]: https://matklad.github.io/2021/09/04/fast-rust-builds.html#Why-Care-About-Build-Times
[developer-productivity]: https://www.computer.org/csdl/magazine/so/2023/04/10176199/1OAJyfknInm
[monomorphization]: https://rustc-dev-guide.rust-lang.org/backend/monomorph.html

#### [`crates/ironrdp`](./crates/ironrdp)

Meta crate re-exporting important crates.

**Architectural Invariant**: this crate re-exports other crates and does not provide anything else.

#### [`crates/ironrdp-core`](./crates/ironrdp-core)

Common traits and types.

This crate is motivated by the fact that only a few items are required to build most of the other crates such as the virtual channels.
To move up these crates up in the compilation tree, `ironrdp-core` must remain small, with very few dependencies.
It contains the most "low-context" building blocks.

Most notable traits are `Decode` and `Encode` which are used to define a common interface for PDU encoding and decoding.
These are object-safe, and must remain so.

Most notable types are `ReadCursor`, `WriteCursor` and `WriteBuf` which are used pervasively for encoding and decoding in a `no-std` manner.

#### [`crates/ironrdp-pdu`](./crates/ironrdp-pdu)

PDU encoding and decoding.

_TODO_: clean up the dependencies

#### [`crates/ironrdp-graphics`](./crates/ironrdp-graphics)

Image processing primitives.

_TODO_: break down into multiple smaller crates

_TODO_: clean up the dependencies

#### [`crates/ironrdp-svc`](./crates/ironrdp-svc)

Traits to implement RDP static virtual channels.

#### [`crates/ironrdp-dvc`](./crates/ironrdp-dvc)

DRDYNVC static channel implementation and traits to implement dynamic virtual channels.

#### [`crates/ironrdp-cliprdr`](./crates/ironrdp-cliprdr)

CLIPRDR static channel for clipboard implemented as described in MS-RDPECLIP.

#### [`crates/ironrdp-rdpdr`](./crates/ironrdp-rdpdr)

RDPDR channel implementation.

#### [`crates/ironrdp-rdpsnd`](./crates/ironrdp-rdpsnd)

RDPSND static channel for audio output implemented as described in MS-RDPEA.

#### [`crates/ironrdp-connector`](./crates/ironrdp-connector)

State machines to drive an RDP connection sequence.

#### [`crates/ironrdp-session`](./crates/ironrdp-session)

State machines to drive an RDP session.

#### [`crates/ironrdp-input`](./crates/ironrdp-input)

Utilities to manage and build input packets.

#### [`crates/ironrdp-rdcleanpath`](./crates/ironrdp-rdcleanpath)

RDCleanPath PDU structure used by IronRDP web client and Devolutions Gateway.

#### [`crates/ironrdp-error`](./crates/ironrdp-error)

Lightweight and `no_std`-compatible generic `Error` and `Report` types.
The `Error` type wraps a custom consumer-defined type for domain-specific details (such as `PduErrorKind`).

#### [`crates/ironrdp-propertyset`](./crates/ironrdp-propertyset)

The main type is `PropertySet`, a key-value store for configuration options.

#### [`crates/ironrdp-rdpfile`](./crates/ironrdp-rdpfile)

Loader and writer for the .RDP file format.

### Extra Tier

Higher level libraries and binaries built on top of the core tier.
Guidelines and constraints are relaxed to some extent.

#### [`crates/ironrdp-blocking`](./crates/ironrdp-blocking)

Blocking I/O abstraction wrapping the state machines conveniently.

This crate is an **API Boundary**.

#### [`crates/ironrdp-async`](./crates/ironrdp-async)

Provides `Future`s wrapping the state machines conveniently.

This crate is an **API Boundary**.

#### [`crates/ironrdp-tokio`](./crates/ironrdp-tokio)

`Framed*` traits implementation above `tokio`’s traits.

This crate is an **API Boundary**.

#### [`crates/ironrdp-futures`](./crates/ironrdp-futures)

`Framed*` traits implementation above `futures`’s traits.

This crate is an **API Boundary**.

#### [`crates/ironrdp-tls`](./crates/ironrdp-tls)

TLS boilerplate common with most IronRDP clients.

NOTE: it’s not yet clear if this crate is an API Boundary or an implementation detail for the native clients.

#### [`crates/ironrdp-client`](./crates/ironrdp-client)

Portable RDP client without GPU acceleration.

#### [`crates/ironrdp-web`](./crates/ironrdp-web)

WebAssembly high-level bindings targeting web browsers.

This crate is an **API Boundary** (WASM module).

#### [`web-client/iron-remote-desktop`](./web-client/iron-remote-desktop)

Core frontend UI used by `iron-svelte-client` as a Web Component.

This crate is an **API Boundary**.

#### [`web-client/iron-remote-desktop-rdp`](./web-client/iron-remote-desktop-rdp)

Implementation of the TypeScript interfaces exposed by WebAssembly bindings from `ironrdp-web` and used by `iron-svelte-client`.

This crate is an **API Boundary**.

#### [`web-client/iron-svelte-client`](./web-client/iron-svelte-client)

Web-based frontend using `Svelte` and `Material` frameworks.

#### [`crates/ironrdp-cliprdr-native`](./crates/ironrdp-cliprdr-native)

Native CLIPRDR backend implementations.

#### [`crates/ironrdp-cfg`](./crates/ironrdp-cfg)

IronRDP-related utilities for ironrdp-propertyset.

### Internal Tier

Crates that are only used inside the IronRDP project, not meant to be published.
This is mostly test case generators, fuzzing oracles, build tools, and so on.

**Architecture Invariant**: these crates are not, and will never be, an **API Boundary**.

#### [`crates/ironrdp-pdu-generators`](./crates/ironrdp-pdu-generators)

`proptest` generators for `ironrdp-pdu` types.

#### [`crates/ironrdp-session-generators`](./crates/ironrdp-session-generators)

`proptest` generators for `ironrdp-session` types.

#### [`crates/ironrdp-testsuite-core`](./crates/ironrdp-testsuite-core)

Contains all integration tests for code living in the core tier, in a single binary, organized in modules.

**Architectural Invariant**: no dependency from another tier is allowed. It must be the case that
compiling and running the core test suite does not require building any library from the extra tier.
This is to keep iteration time short.

#### [`crates/ironrdp-testsuite-extra`](./crates/ironrdp-testsuite-extra)

Contains all integration tests for code living in the extra tier, in a single binary, organized in modules.

#### [`crates/ironrdp-fuzzing`](./crates/ironrdp-fuzzing)

Provides test case generators and oracles for use with fuzzing.

#### [`fuzz`](./fuzz)

Fuzz targets for code in core tier.

#### [`xtask`](./xtask)

IronRDP’s free-form automation using Rust code.

### Community Tier

Crates provided and maintained by the community. Core maintainers will not invest a lot of time into
these. One or several community maintainers are associated to each one.

The IronRDP team is happy to accept new crates but may not necessarily commit to keeping them
working when changing foundational libraries. We promise to notify you if such a crate breaks, and
will always try to fix things when it's a minor change.

#### [`crates/ironrdp-acceptor`](./crates/ironrdp-acceptor) (@mihneabuz)

State machines to drive an RDP connection acceptance sequence

#### [`crates/ironrdp-server`](./crates/ironrdp-server) (@mihneabuz)

Extendable skeleton for implementing custom RDP servers.

#### [`crates/ironrdp-mstsgu`](./crates/ironrdp-mstsgu) (@steffengy)

Terminal Services Gateway Server Protocol implementation.

#### [`crates/ironrdp-glutin-renderer`](./crates/ironrdp-glutin-renderer) (no maintainer)

`glutin` primitives for OpenGL rendering.

#### [`crates/ironrdp-client-glutin`](./crates/ironrdp-client-glutin) (no maintainer)

GPU-accelerated RDP client using glutin.

#### [`crates/ironrdp-replay-client`](./crates/ironrdp-replay-client) (no maintainer)

Utility tool to replay RDP graphics pipeline for debugging purposes.

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

See [`fuzz/README.md`](./fuzz/README.md).

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
