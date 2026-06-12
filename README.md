# IronRDP

[![](https://docs.rs/ironrdp/badge.svg)](https://docs.rs/ironrdp/) [![](https://img.shields.io/crates/v/ironrdp)](https://crates.io/crates/ironrdp)

A collection of Rust crates providing an implementation of the Microsoft Remote Desktop Protocol, with a focus on security.

> [!WARNING]
> ## ⚗️ Experimental branch: `experiment/web-rdp-h264-webgpu`
>
> **This branch is a proof-of-concept for experimentation only — not production-ready and not intended to be merged as-is.** It demonstrates a full **MS-RDPEGFX H.264/AVC graphics pipeline running in a browser**, end to end: a plugin-free web page driving a real RDP session with **hardware H.264 decode (WebCodecs)** presented through **WebGPU**.
>
> ### What it changes relative to `master`
> - **`ironrdp-connector`** — advertises `SUPPORT_DYN_VC_GFX_PROTOCOL` in the client core data, so the server enables the RDP 8.0+ EGFX graphics pipeline instead of classifying the client as RDP ≤7.1 and falling back to RemoteFX.
> - **`ironrdp-egfx`** — implements **AVC444** decode (extracts the AVC420 main view) alongside AVC420, via a shared dispatch path.
> - **`ironrdp-web`** — new `egfx` module: hardware **H.264 decode via the browser `VideoDecoder` (WebCodecs)**, composited/presented on **WebGPU** through the [`softblit`](https://github.com/irvingoujAtDevolution/softblit) presenter with a **zero-copy GPU path** (`VideoFrame` → `copyExternalImageToTexture`). NAL framing is auto-detected (Annex-B vs length-prefixed). Advertises EGFX capability versions V10.7→V10.4 + V8.1.
> - **web demo (`iron-svelte-client`)** — `.env`-driven login prefill, a resolution picker including **"match display"**, and a native **Fullscreen** mode.
>
> ### What's possible with it
> A browser tab running a full RDP desktop with hardware-decoded H.264 — smooth full-motion video (no RemoteFX band-fill tearing) and a native-feeling fullscreen session, all from a Rust→WASM RDP stack. This is the same codec family Azure Virtual Desktop / Windows 365 use.
>
> ### Requirements & caveats
> - **Server must emit H.264/AVC**: Windows 10/11 or Server 2016+ with a WDDM graphics path (a GPU-less *Gen1* Hyper-V VM cannot). On the host, enable **"Prioritize H.264/AVC 444 Graphics mode"** and set **"Configure image quality for RemoteFX Adaptive Graphics" = High** (Lossless disables H.264).
> - **Browser** must support **WebGPU + WebCodecs** (Chromium-based). The web build requires `--cfg=web_sys_unstable_apis` (handled by `cargo xtask web build`).
> - Depends on the experimental [`softblit`](https://github.com/irvingoujAtDevolution/softblit) WebGPU presenter (pinned git dependency in `crates/ironrdp-web/Cargo.toml`).
> - Chroma is currently **4:2:0** — we decode only the AVC444 *main* view and drop the chroma-upgrade aux stream. True 4:4:4 is a future step.

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
