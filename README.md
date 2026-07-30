<h1 align="center">IronRDP</h1>

<p align="center">
  <strong>A Rust implementation of the Microsoft Remote Desktop Protocol, with a focus on security.</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/ironrdp"><img src="https://img.shields.io/crates/v/ironrdp?logo=rust" alt="crates.io"></a>
  <a href="https://docs.rs/ironrdp/"><img src="https://docs.rs/ironrdp/badge.svg" alt="docs.rs"></a>
  <a href="https://github.com/Devolutions/IronRDP/actions/workflows/ci.yml"><img src="https://github.com/Devolutions/IronRDP/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License: MIT OR Apache-2.0">
  <a href="https://matrix.to/#/#IronRDP:matrix.org"><img src="https://img.shields.io/badge/chat-matrix-brightgreen?logo=matrix" alt="Matrix"></a>
</p>

IronRDP is a modular suite of Rust crates implementing RDP, the protocol behind Windows Remote
Desktop. It is not a single monolithic client: it is a set of composable building blocks — PDU
encoding/decoding, connection and session state machines, virtual channels, image codecs — that you
can assemble into a client, a server, or a proxy, on native platforms, in the browser via
WebAssembly, or from .NET through FFI bindings.

The core protocol crates do no I/O, are `no_std`-compatible, and are continuously fuzzed. You bring
the transport and the runtime; IronRDP brings the protocol.

## Demonstration

https://user-images.githubusercontent.com/3809077/202049929-76f42471-aeb0-41da-9118-0dc6ea491bd2.mp4

## Highlights

- **Sans-I/O core.** Connection and session logic are state machines with no sockets, no threads, and
  no runtime attached. Drive them with blocking I/O, `tokio`, `futures`, or your own event loop.
- **Security first.** Parsing is treated as a hostile-input surface: every core crate is fuzzed,
  `unsafe` is heavily linted, and the workspace enforces a strict correctness lint policy.
- **Runs everywhere.** Native binaries, a WebAssembly module for browsers, and C#/.NET bindings all
  build from the same protocol core.
- **Client *and* server.** Use the connector to talk to a Windows host, or the acceptor and server
  skeleton to expose your own desktop over RDP.
- **Pick only what you need.** Every subsystem is a separate crate behind a feature flag, so a
  headless screenshot tool does not pull in audio, clipboard, or a GUI stack.

## Features

**Protocol and security**

- RDP connection sequence: X.224 negotiation, MCS, capability exchange, licensing, reactivation
- Enhanced RDP Security with TLS (1.2 and 1.3)
- Network Level Authentication (NLA) via CredSSP, with NTLM and Kerberos
- KDC proxy and RDCleanPath support for gateway-mediated, just-in-time connections
- Terminal Services Gateway (MS-TSGU) transport
- `.rdp` file parsing and writing, plus a typed configuration property store

**Graphics**

- Client-side decoding: uncompressed raw bitmaps, Interleaved RLE, RDP 6.0 bitmap compression, and
  RemoteFX (RFX)
- Server-side encoding: RDP 6.0 bitmap compression, RemoteFX, optional NSCodec, and optional
  QOI / QOI+zstd
- Additional codec primitives available as libraries: ClearCodec, RemoteFX Progressive, ZGFX, and
  the graphics pipeline (EGFX) PDUs
- Bulk compression: MPPC, NCRUSH, and XCRUSH

**Virtual channels**

- Static (SVC) and dynamic (DVC / DRDYNVC) channel infrastructure
- Clipboard redirection (CLIPRDR), audio output (RDPSND), device and smart card redirection (RDPDR)
- Display control for dynamic resizing, echo (RTT probes), alternative input, and USB redirection
- Windows DVC COM plugin loader and a DVC named-pipe proxy for bridging external processes

**Targets and bindings**

- Native clients on Windows, macOS, and Linux
- WebAssembly bindings plus a protocol-agnostic web component (`@devolutions/iron-remote-desktop`)
- C#/.NET bindings generated with [Diplomat] (`Devolutions.IronRdp`)

## Getting started

### Prebuilt binaries

Checksummed `.tar.gz` archives are attached to each GitHub Release, one per supported platform:

- [`ironrdp-viewer`](./crates/ironrdp-viewer) — a windowed RDP client (tags `ironrdp-viewer-v*`)
- [`ironrdp-agent`](./crates/ironrdp-agent) — a daemon-backed CLI for automation (tags `ironrdp-agent-v*`)

Download, checksum, and extraction instructions are included in each release's notes on the
[Releases page](https://github.com/Devolutions/IronRDP/releases).

### Install with Cargo

```shell
cargo install ironrdp-viewer
cargo install ironrdp-agent
```

### `ironrdp-viewer`

`ironrdp-viewer` is a portable, windowed RDP client that uses asynchronous I/O and software
rendering.

```shell
ironrdp-viewer <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

Omitted credentials are prompted for interactively. You can also load a `.rdp` file:

```shell
ironrdp-viewer --rdp-file ./my-server.rdp
```

Set `IRONRDP_LOG` to adjust logging, e.g. `IRONRDP_LOG="info,ironrdp_connector=trace"`. See the
[viewer README](./crates/ironrdp-viewer/README.md) for the supported `.rdp` properties, TLS key
logging, and the full option list.

### `ironrdp-agent`

`ironrdp-agent` combines a long-lived RDP daemon with short-lived CLI invocations, which makes it
convenient for scripts and LLM-driven automation:

```shell
ironrdp-agent daemon-start                                    # in one terminal
ironrdp-agent connect --server <HOSTNAME> --username <USER>   # in another
ironrdp-agent screenshot ./desktop.png
```

Run `ironrdp-agent --help-agent` for a machine-readable description of every operation, and see the
[agent README](./crates/ironrdp-agent/README.md) for the IPC format, secret handling, and remote
execution support.

## Using IronRDP as a library

Add the meta crate and enable only the pieces you need:

```toml
[dependencies]
ironrdp = { version = "0.17", features = ["connector", "session", "graphics"] }
```

Each feature maps to a standalone crate, so you can also depend on `ironrdp-pdu`,
`ironrdp-connector`, `ironrdp-session`, and friends directly. API documentation lives on
[docs.rs](https://docs.rs/ironrdp/).

Two runnable examples ship with the meta crate:

```shell
# Connect, decode the desktop, and write a PNG. Blocking, synchronous I/O.
cargo run --example=screenshot -- --host <HOSTNAME> -u <USERNAME> -p <PASSWORD> -o out.png

# A minimal RDP server built on ironrdp-server.
cargo run --example=server -- --bind-addr 127.0.0.1:3389
```

## Building from source

### Prerequisites

- The Rust toolchain pinned in [`rust-toolchain.toml`](./rust-toolchain.toml) (installed
  automatically by `rustup`)
- Node.js >= 24 LTS, for the web client only
- The .NET SDK, for the FFI bindings only

### Build and check

```shell
git clone https://github.com/Devolutions/IronRDP.git
cd IronRDP
cargo build
```

Project automation lives in [`xtask`](./xtask), following the
[`cargo xtask`](https://github.com/matklad/cargo-xtask) convention. Run `cargo xtask --help` for the
full list. The most useful ones:

```shell
cargo xtask bootstrap        # install development requirements
cargo xtask check fmt        # formatting
cargo xtask check lints      # clippy
cargo xtask check tests      # test suites
cargo xtask ci               # everything CI runs
```

A successful `cargo xtask ci` locally is expected to imply a green CI run.

### Web client

```shell
cargo xtask web install
cargo xtask web run
```

This builds the WebAssembly module and serves the SvelteKit demonstration client. See
[`web-client/`](./web-client) for details. The demo client is a showcase, not a production build.

### .NET bindings

```shell
cargo xtask ffi install
cargo xtask ffi build
cargo xtask ffi bindings
```

Then run one of the samples, e.g. `dotnet run --project ffi/dotnet/Devolutions.IronRdp.ConnectExample`.
See [`ffi/`](./ffi) for details.

### Fuzzing

```shell
cargo xtask fuzz install
cargo xtask fuzz run
```

See [`fuzz/README.md`](./fuzz/README.md).

## Repository layout

| Path | Contents |
| --- | --- |
| [`crates/`](./crates) | The crate suite: protocol, channels, codecs, client, server, bindings |
| [`crates/ironrdp`](./crates/ironrdp) | Meta crate re-exporting the others, plus runnable examples |
| [`web-client/`](./web-client) | Web component, RDP backend, and Svelte demonstration client |
| [`ffi/`](./ffi) | Diplomat-based FFI and .NET bindings with examples |
| [`fuzz/`](./fuzz) | Fuzz targets for the core tier |
| [`benches/`](./benches) | Benchmarks |
| [`xtask/`](./xtask) | Project automation |

Crates are organized into core, extra, internal, and community tiers, each with its own guarantees
and invariants. Read [ARCHITECTURE.md](./ARCHITECTURE.md) before making non-trivial changes, and
[STYLE.md](./STYLE.md) for coding conventions.

## Who uses IronRDP

- [Devolutions Gateway](https://github.com/Devolutions/devolutions-gateway) and its free standalone
  web interface, as well as Devolutions Server, Devolutions Hub, and Remote Desktop Manager
- [Teleport](https://github.com/gravitational/teleport), for its remote desktop access
- [`qemu-rdp`](https://gitlab.com/marcandre.lureau/qemu-display), an RDP server for QEMU displays
- A growing set of community projects building RDP servers and clients on top of the crate suite

## Rust version (MSRV)

IronRDP libraries follow a conservative Minimum Supported Rust Version policy. The MSRV is the oldest
stable Rust release that is at least 6 months old, bounded by the Rust version available in
[Debian stable-backports](https://packages.debian.org/search?suite=all&arch=any&searchon=names&keywords=rust)
and [Fedora stable](https://packages.fedoraproject.org/pkgs/rust/rust/). The toolchain pinned in
`rust-toolchain.toml` is both the project toolchain and the MSRV validated by CI. See
[ARCHITECTURE.md](./ARCHITECTURE.md#msrv-policy) for the full policy.

## Contributing

Contributions are welcome. Start with [ARCHITECTURE.md](./ARCHITECTURE.md) and
[STYLE.md](./STYLE.md), keep changes scoped, and make sure `cargo xtask ci` passes before opening a
pull request.

## Getting help

- Report bugs in the [issue tracker](https://github.com/Devolutions/IronRDP/issues)
- Discuss the project in the [Matrix room](https://matrix.to/#/#IronRDP:matrix.org)

## License

Licensed under either of [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE) at your option.

[Diplomat]: https://github.com/rust-diplomat/diplomat
