# IronRDP

A Rust implementation of the Microsoft Remote Desktop Protocol, with a focus on security.

## Architecture (Work In Progress…)

- `ironrdp`: meta crate re-exporting important crates
- `ironrdp-core`: core, RDP protocol packets encoding and decoding.
- `ironrdp-graphics`: image processing primitives and algorithms (ZGFX, DWT…).
- `ironrdp-input`: helpers to build FastPathInput packets.
- `ironrdp-session`: abstract state machine on top of `ironrdp-graphics`.
- `ironrdp-session-async`: `Future`s built on top of `ironrdp-session`.
- `ironrdp-tls`: TLS boilerplate common with most IronRDP clients.
- `ironrdp-devolutions-gateway`: Devolutions Gateway extensions.
- `ironrdp-renderer`: `glutin` primitives for OpenGL rendering.
- `ironrdp-cli`: basic command-line client mostly for debugging purposes.
- `ironrdp-gui-client`: basic GUI client for IronRDP.
- `ironrdp-replay-client`: utility tool to replay RDP graphics pipeline for debugging purposes.
- `iron-remote-gui`: core frontend ui used by both, iron-svelte-client and iron-tauri-client.
- `iron-svelte-client`: web-based frontend using `Svelte` and `Material` frameworks).
- `iron-tauri-client`: a native client built with Tauri. Frontend is using the `iron-web-client`/`iron-svelte-client` component.
- `ffi/wasm`: WebAssembly high-level bindings targeting web browsers.

## Video Codec Support

Currently, only Microsoft RemoteFX (RFX) is supported.

In order to enable RemoteFX:

1. Run `gpedit.msc`.

2. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/RemoteFX for Windows Server 2008 R2/Configure RemoteFX`

3. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Enable RemoteFX encoding for RemoteFX clients designed for Windows Server 2008 R2 SP1`

4. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Limit maximum color depth`

5. Reboot.


## Demonstration

https://user-images.githubusercontent.com/3809077/202049929-76f42471-aeb0-41da-9118-0dc6ea491bd2.mp4

