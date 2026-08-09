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

IronRDP is a modular Rust implementation of RDP, the protocol behind Windows Remote Desktop.
It is not a monolithic client: its composable crates provide PDU codecs, connection and session state machines, virtual channels, and image codecs for native, WebAssembly, and .NET clients, servers, and proxies.
The continuously fuzzed, `no_std`-compatible core performs no I/O, so applications supply the transport and runtime.

## Highlights

- **Sans-I/O core:** Drive connection and session state machines with blocking I/O, `tokio`, `futures`, or your own event loop.
- **Security first:** Core crates are fuzzed, `unsafe` is heavily linted, and the workspace enforces strict correctness lints.
- **Runs everywhere:** The same protocol core powers native binaries, WebAssembly modules, and C#/.NET bindings.
- **Client and server:** Connect to Windows hosts or expose your own desktop through the acceptor and server skeleton.
- **Modular:** Enable only the crates and features you need without pulling in unrelated subsystems.

## Features

**Protocol and security**

- RDP connection sequence: X.224 negotiation, MCS, capability exchange, licensing, reactivation
- Enhanced RDP Security with TLS (1.2 and 1.3)
- Network Level Authentication (NLA) via CredSSP, with NTLM and Kerberos
- KDC proxy and RDCleanPath support for gateway-mediated, just-in-time connections
- Terminal Services Gateway (MS-TSGU) transport
- `.rdp` file parsing and writing, plus a typed configuration property store

**Graphics**

- Client-side decoding: uncompressed raw bitmaps, Interleaved RLE, RDP 6.0 bitmap compression, and RemoteFX (RFX)
- Server-side encoding: RDP 6.0 bitmap compression, RemoteFX, optional NSCodec, and optional QOI / QOI+zstd
- Additional codec primitives available as libraries: ClearCodec, RemoteFX Progressive, ZGFX, and the graphics pipeline (EGFX) PDUs
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

Checksummed `.tar.gz` archives are attached to each GitHub release, one per supported platform:

- [`ironrdp-viewer`][ironrdp-viewer] - a windowed RDP client (tags `ironrdp-viewer-v*`)
- [`ironrdp-agent`][ironrdp-agent] - a daemon-backed CLI for automation (tags `ironrdp-agent-v*`)

Each [release][releases] includes download, checksum, and extraction instructions.

### Install with Cargo

```shell
cargo install ironrdp-viewer
cargo install ironrdp-agent
```

Both binaries link native audio, so Linux builds need the ALSA development headers (`libasound2-dev` on Debian/Ubuntu) and Windows builds need NASM.

### `ironrdp-viewer`

`ironrdp-viewer` is a portable, windowed RDP client with asynchronous I/O and software rendering.

```shell
ironrdp-viewer <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

Omitted credentials are prompted for interactively.
You can also load a `.rdp` file:

```shell
ironrdp-viewer --rdp-file ./my-server.rdp
```

Set `IRONRDP_LOG` to adjust logging, for example `IRONRDP_LOG="info,ironrdp_connector=trace"`.
See the [viewer README] for supported `.rdp` properties, TLS key logging, and the full option list.

### `ironrdp-agent`

`ironrdp-agent` combines a long-lived RDP daemon with short-lived CLI invocations for scripts and LLM-driven automation:

```shell
ironrdp-agent daemon-start --overlay ./credentials.rdp        # in one terminal
ironrdp-agent connect --server <HOSTNAME> --username <USER>   # in another
ironrdp-agent screenshot ./desktop.png
```

`connect` fails with `missing required fields` unless credentials are available.
Preload credentials with `daemon-start --overlay <FILE>` to keep secrets away from the IPC caller, or pass `--password` to `connect`.
Run `ironrdp-agent --help-agent` for a machine-readable description of every operation.
See the [agent README] for the IPC format, secret handling, and remote execution support.

## Using IronRDP as a library

Add the meta crate and enable only the pieces you need:

```toml
[dependencies]
ironrdp = { version = "0.17", features = ["connector", "session", "graphics"] }
```

Each feature maps to a standalone crate, so you can depend on `ironrdp-pdu`, `ironrdp-connector`, `ironrdp-session`, and related crates directly.
API documentation is available on [docs.rs].

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

Alternatively, enable these group policies with `gpedit.msc` and reboot.
They are under _Computer Configuration → Administrative Templates → Windows Components → Remote Desktop Services → Remote Desktop Session Host → Remote Session Environment_:

1. _RemoteFX for Windows Server 2008 R2 → Configure RemoteFX_
2. _Enable RemoteFX encoding for RemoteFX clients designed for Windows Server 2008 R2 SP1_
3. _Limit maximum color depth_

</details>

## Who uses IronRDP

- [Devolutions Gateway] for browser-based and native RDP client access
- [Cloudflare Access] for browser-based RDP
- [Teleport] for remote desktop web access
- [Lamco RDP Server], a Wayland-native RDP server for Linux desktop sharing
- [MacRDP], a native RDP server for macOS
- [`qemu-rdp`][qemu-rdp], an RDP server for QEMU displays
- A growing set of community projects building RDP servers and clients on the crate suite

## Rust version (MSRV)

IronRDP's MSRV is the oldest of three versions: the latest stable Rust release at least six months old, the version packaged by [Fedora stable], and the version available in [Debian stable-backports].
`rust-toolchain.toml` pins both the project toolchain and the MSRV validated by CI.
See the [architecture policy] for details.

## Contributing

Contributions are welcome; start with [ARCHITECTURE] and [STYLE], and keep changes scoped.
Project automation uses [`xtask`][xtask] following the [`cargo xtask`][cargo xtask] convention.
Run `cargo xtask --help` for the command list and `cargo xtask bootstrap` to install development requirements.
Run `cargo xtask ci` before opening a pull request; it covers everything CI runs except FFI and .NET checks, which have separate `cargo xtask ffi` commands.

Workspace builds use the native prerequisites listed under [Install with Cargo].
The web client also needs Node.js >= 24 LTS, and the FFI bindings need the .NET SDK.

## AI-assisted development

AI-assisted development is welcome, but contributors remain responsible for understanding, reviewing, and validating every change.
For RDP protocol work, install the Windows Protocols skill from [awakecoding/openspecs] so agents can navigate the Microsoft Open Specifications corpus.

## Getting help

- Report bugs in the [issue tracker]
- Discuss the project in the [Matrix room]

## License

Licensed under either [MIT] or [Apache-2.0] at your option.

[ironrdp-viewer]: ./crates/ironrdp-viewer
[ironrdp-agent]: ./crates/ironrdp-agent
[releases]: https://github.com/Devolutions/IronRDP/releases
[viewer README]: ./crates/ironrdp-viewer/README.md
[agent README]: ./crates/ironrdp-agent/README.md
[docs.rs]: https://docs.rs/ironrdp/
[Diplomat]: https://github.com/rust-diplomat/diplomat
[Devolutions Gateway]: https://github.com/Devolutions/devolutions-gateway
[Cloudflare Access]: https://blog.cloudflare.com/browser-based-rdp/
[Teleport]: https://goteleport.com/
[Lamco RDP Server]: https://lamco.ai/products/lamco-rdp-server/
[MacRDP]: https://github.com/clintcan/macrdp
[qemu-rdp]: https://gitlab.com/marcandre.lureau/qemu-display
[Debian stable-backports]: https://packages.debian.org/search?suite=all&arch=any&searchon=names&keywords=rust
[Fedora stable]: https://packages.fedoraproject.org/pkgs/rust/rust/
[architecture policy]: ./ARCHITECTURE.md#msrv-policy
[ARCHITECTURE]: ./ARCHITECTURE.md
[STYLE]: ./STYLE.md
[xtask]: ./xtask
[cargo xtask]: https://github.com/matklad/cargo-xtask
[Install with Cargo]: #install-with-cargo
[awakecoding/openspecs]: https://github.com/awakecoding/openspecs
[issue tracker]: https://github.com/Devolutions/IronRDP/issues
[Matrix room]: https://matrix.to/#/#IronRDP:matrix.org
[MIT]: ./LICENSE-MIT
[Apache-2.0]: ./LICENSE-APACHE
