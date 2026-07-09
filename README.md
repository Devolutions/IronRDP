# IronRDP

[![](https://docs.rs/ironrdp/badge.svg)](https://docs.rs/ironrdp/) [![](https://img.shields.io/crates/v/ironrdp)](https://crates.io/crates/ironrdp)

A collection of Rust crates providing an implementation of the Microsoft Remote Desktop Protocol, with a focus on security.

## Demonstration

<https://user-images.githubusercontent.com/3809077/202049929-76f42471-aeb0-41da-9118-0dc6ea491bd2.mp4>

## Video Codec Support

Supported codecs:

- Uncompressed raw bitmap
- Interleaved Run-Length Encoding (RLE) Bitmap Codec
- RDP 6.0 Bitmap Compression
- Microsoft RemoteFX (RFX)

## Examples

### [`ironrdp-viewer`](https://github.com/Devolutions/IronRDP/tree/master/crates/ironrdp-viewer)

A full-fledged RDP client based on IronRDP crates suite, and implemented using non-blocking, asynchronous I/O.
It is built on top of the reusable [`ironrdp-client`](https://github.com/Devolutions/IronRDP/tree/master/crates/ironrdp-client) library crate.

```shell
cargo run --bin ironrdp-viewer -- <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

### [`screenshot`](https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp/examples/screenshot.rs)

Example of utilizing IronRDP in a blocking, synchronous fashion.

This example showcases the use of IronRDP in a blocking manner. It
demonstrates how to create a basic RDP client with just a few hundred lines
of code by leveraging the IronRDP crates suite.

In this basic client implementation, the client establishes a connection
with the destination server, decodes incoming graphics updates, and saves the
resulting output as a BMP image file on the disk.

```shell
cargo run --example=screenshot -- --host <HOSTNAME> --username <USERNAME> --password <PASSWORD> --output out.bmp
```

### How to enable RemoteFX on server

Run the following PowerShell commands, and reboot.

```pwsh
Set-ItemProperty -Path 'HKLM:\Software\Policies\Microsoft\Windows NT\Terminal Services' -Name 'ColorDepth' -Type DWORD -Value 5
Set-ItemProperty -Path 'HKLM:\Software\Policies\Microsoft\Windows NT\Terminal Services' -Name 'fEnableVirtualizedGraphics' -Type DWORD -Value 1
```

Alternatively, you may change a few group policies using `gpedit.msc`:

1. Run `gpedit.msc`.

2. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/RemoteFX for Windows Server 2008 R2/Configure RemoteFX`

3. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Enable RemoteFX encoding for RemoteFX clients designed for Windows Server 2008 R2 SP1`

4. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Limit maximum color depth`

5. Reboot.

## Binary releases

Standalone archives are attached to GitHub Releases for the executable packages:

- [`ironrdp-agent`](./crates/ironrdp-agent) provides the agentic, daemon-backed CLI.
- [`ironrdp-viewer`](./crates/ironrdp-viewer) provides the windowed RDP client CLI.

Each release provides one `.tar.gz` archive and a SHA-256 sidecar for these native target triples:

| Platform | Target triple |
| --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` |
| Windows ARM64 | `aarch64-pc-windows-msvc` |
| Linux x64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` |
| macOS x64 | `x86_64-apple-darwin` |
| macOS ARM64 | `aarch64-apple-darwin` |

Linux archives use an Ubuntu 22.04 build baseline and require glibc 2.35 or later. macOS archives
target macOS 10.13 or later on Intel and macOS 11.0 or later on Apple Silicon.

For example, download and extract the Linux x64 agent from its release:

```shell
VERSION=<VERSION>
ASSET="ironrdp-agent-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
curl -fLO "https://github.com/Devolutions/IronRDP/releases/download/ironrdp-agent-v${VERSION}/${ASSET}"
curl -fLO "https://github.com/Devolutions/IronRDP/releases/download/ironrdp-agent-v${VERSION}/${ASSET}.sha256"
sha256sum --check "${ASSET}.sha256"
tar -xzf "${ASSET}"
```

Replace `ironrdp-agent` with `ironrdp-viewer` to download the windowed client from its corresponding
package release. Windows archives contain an `.exe`; all other archives contain the executable without
an extension.

## Rust version (MSRV)

IronRDP libraries follow a conservative Minimum Supported Rust Version (MSRV) policy.
The MSRV is the oldest stable Rust release that is at least 6 months old, bounded by the Rust version available in [Debian stable-backports](https://packages.debian.org/search?suite=all&arch=any&searchon=names&keywords=rust) and [Fedora stable](https://packages.fedoraproject.org/pkgs/rust/rust/).
The pinned toolchain in `rust-toolchain.toml` is both the project toolchain and the MSRV validated by CI.
See [ARCHITECTURE.md](./ARCHITECTURE.md#msrv-policy) for the full policy.

## Architecture

See the [ARCHITECTURE.md](https://github.com/Devolutions/IronRDP/blob/master/ARCHITECTURE.md) document.

## Getting help

- Report bugs in the [issue tracker](https://github.com/Devolutions/IronRDP/issues)
- Discuss the project on the [matrix room](https://matrix.to/#/#IronRDP:matrix.org)
