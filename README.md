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

Both binaries link native audio, so Linux builds need the ALSA development headers
(`libasound2-dev` on Debian/Ubuntu) and Windows builds need NASM.

### `ironrdp-viewer`

`ironrdp-viewer` is a portable, windowed RDP client that uses asynchronous I/O and software
rendering.

```shell
ironrdp-viewer <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

Omitted connection values are prompted for interactively. You can also provide them through
`RDP_HOSTNAME`, `RDP_USERNAME`, and `RDP_PASSWORD`, or load a `.rdp` file:

```shell
ironrdp-viewer --rdp-file ./my-server.rdp
```

Set `IRONRDP_LOG` to adjust logging, e.g. `IRONRDP_LOG="info,ironrdp_connector=trace"`. See the
[viewer README](./crates/ironrdp-viewer/README.md) for the supported `.rdp` properties, TLS key
logging, and the full option list.

### `ironrdp-agent`

`ironrdp-agent` combines a long-lived local RPC backend with short-lived CLI invocations, which makes
it convenient for scripts and LLM-driven automation. The daemon backend remains the default; use
`--backend viewer` to drive a visible `ironrdp-viewer` window instead:

```shell
ironrdp-agent daemon-start --overlay ./credentials.rdp        # in one terminal
ironrdp-agent connect --server <HOSTNAME> --username <USER>   # in another
ironrdp-agent screenshot ./desktop.png
ironrdp-agent --backend viewer connect --server <HOSTNAME> --username <USER> --password <PASS>
```

Normal operations automatically start and reuse the selected backend. The daemon and viewer use
distinct per-user endpoints by default; `--endpoint` overrides either one. Run
`ironrdp-agent --backend viewer stop` (or `stop` with the default backend) to terminate the selected
long-lived process. The viewer backend implements the same status, input, screenshot, and NOW
operations while rendering the shared session in its GUI.

`connect` fails with `missing required fields` unless credentials are available, so either preload
them into the daemon with `daemon-start --overlay <FILE>` — which keeps secrets away from the IPC
caller — or pass `--password` to `connect`.

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

## Tips

<details>
<summary>Enabling RemoteFX on a Windows server</summary>

Run the following PowerShell commands, then reboot:

```pwsh
Set-ItemProperty -Path 'HKLM:\Software\Policies\Microsoft\Windows NT\Terminal Services' -Name 'ColorDepth' -Type DWORD -Value 5
Set-ItemProperty -Path 'HKLM:\Software\Policies\Microsoft\Windows NT\Terminal Services' -Name 'fEnableVirtualizedGraphics' -Type DWORD -Value 1
```

Alternatively, enable the following group policies with `gpedit.msc` and reboot. All of them live
under _Computer Configuration → Administrative Templates → Windows Components → Remote Desktop
Services → Remote Desktop Session Host → Remote Session Environment_:

1. _RemoteFX for Windows Server 2008 R2 → Configure RemoteFX_
2. _Enable RemoteFX encoding for RemoteFX clients designed for Windows Server 2008 R2 SP1_
3. _Limit maximum color depth_

</details>

## Who uses IronRDP

- [Devolutions Gateway](https://github.com/Devolutions/devolutions-gateway) for browser-based and
  native RDP client access
- [Cloudflare Access](https://blog.cloudflare.com/browser-based-rdp/) for browser-based RDP
- [Teleport](https://goteleport.com/) for its remote desktop web access
- [Lamco RDP Server](https://lamco.ai/products/lamco-rdp-server/), a Wayland-native RDP server for
  Linux desktop sharing
- [MacRDP](https://github.com/clintcan/macrdp), a native RDP server for macOS
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
[STYLE.md](./STYLE.md), and keep changes scoped.

Project automation lives in [`xtask`](./xtask), following the
[`cargo xtask`](https://github.com/matklad/cargo-xtask) convention. Run `cargo xtask --help` for the
full list, `cargo xtask bootstrap` to install the development requirements, and `cargo xtask ci`
before opening a pull request — it runs everything CI does except the FFI and .NET checks, which
have their own `cargo xtask ffi` commands.

Building the workspace needs the ALSA development headers on Linux (`libasound2-dev` on
Debian/Ubuntu) and NASM on Windows. The web client additionally needs Node.js >= 24 LTS, and the FFI
bindings need the .NET SDK.

## AI-assisted development

AI-assisted development is encouraged when used thoughtfully. Contributors remain responsible for
understanding, reviewing, and validating every change produced with AI assistance.

For RDP protocol work, install the Windows Protocols skill so AI agents can navigate the Microsoft
Open Specifications corpus. It significantly improves the correctness of AI-assisted work involving
RDP protocol details. See [awakecoding/openspecs](https://github.com/awakecoding/openspecs) for
installation instructions.

## Getting help

- Report bugs in the [issue tracker](https://github.com/Devolutions/IronRDP/issues)
- Discuss the project in the [Matrix room](https://matrix.to/#/#IronRDP:matrix.org)

## License

Licensed under either of [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE) at your option.

[Diplomat]: https://github.com/rust-diplomat/diplomat
