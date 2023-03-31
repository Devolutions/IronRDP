# IronRDP

A collection of Rust crates providing an implementation of the Microsoft Remote Desktop Protocol, with a focus on security.

## Demonstration

https://user-images.githubusercontent.com/3809077/202049929-76f42471-aeb0-41da-9118-0dc6ea491bd2.mp4

## Video Codec Support

Supported codecs:

- Uncompressed raw bitmap
- Interleaved Run-Length Encoding (RLE) Bitmap Codec
- Microsoft RemoteFX (RFX)

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

## Architecture (Work In Progress…)

- `ironrdp` (root package): meta crate re-exporting important crates,
- `ironrdp-pdu` (`crates/pdu`): PDU encoding and decoding (no I/O, trivial to fuzz),
- `ironrdp-graphics` (`crates/graphics`): image processing primitives (no I/O, trivial to fuzz),
- `ironrdp-session` (`crates/session`): state machines to drive an RDP session (no I/O, not _too_ hard to fuzz),
- `ironrdp-input` (`crates/input`): utilities to manage and build input packets (no I/O),
- `ironrdp-session-async` (`crates/session-async`): provides `Future`s wrapping the session state machines conveniently,
- `ironrdp-tls` (`crates/tls`): TLS boilerplate common with most IronRDP clients,
- `ironrdp-rdcleanpath` (`crates/rdcleanpath`): RDCleanPath PDU structure used by IronRDP web client and Devolutions Gateway,
- `ironrdp-client` (`crates/client`): Portable RDP client without GPU acceleration using softbuffer and winit for windowing,
- `ironrdp-web` (`crates/web`): WebAssembly high-level bindings targeting web browsers,
- `ironrdp-glutin-renderer` (`crates/glutin-renderer`): `glutin` primitives for OpenGL rendering,
- `ironrdp-client-glutin` (`crates/client-glutin`): GPU-accelerated RDP client using glutin,
- `ironrdp-replay-client` (`crates/replay-client`): utility tool to replay RDP graphics pipeline for debugging purposes,
- `ironrdp-pdu-generators` (`crates/pdu-generators`): `proptest` generators for `ironrdp-pdu` types,
- `ironrdp-session-generators` (`crates/session-generators`): `proptest` generators for `ironrdp-session` types,
- `iron-remote-gui` (`web-client/iron-remote-gui`): core frontend UI used by `iron-svelte-client` as a Web Component,
- `iron-svelte-client` (`web-client/iron-svelte-client`): web-based frontend using `Svelte` and `Material` frameworks,
- and finally, `ironrdp-fuzz` (`fuzz`): fuzz targets for core crates.

## General design

- Avoid I/O wherever possible
- Dependency injection when runtime information is necessary in core crates (no system call such as `gethostname`)
- Keep non-portable code out of core crates
- Make crate `no_std`-compatible wherever possible
- Facilitate fuzzing
- In libraries, provide concrete error types either hand-crafted or using `thiserror` crate
- In binaries, use the convenient catch-all error type `anyhow::Error`
- Free-form automation a-la `make` following [`cargo xtask`](https://github.com/matklad/cargo-xtask) specification

## Continuous integration

We use GitHub action and our workflows simply run `cargo xtask`.
The expectation is that, if `cargo xtask ci` passes locally, the CI will be green as well.
