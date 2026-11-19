# FFI capability matrix

The .NET FFI exposes low-level RDP connection and session primitives.
This matrix compares that surface with capabilities in the Rust client crate; it describes coverage, not product demand.

Coverage labels describe the current .NET FFI, and priorities rank capability gaps by breadth rather than expected user demand.
“Absent” means no corresponding FFI API was found, not that Rust supports every platform or deployment.

## Capability matrix

| Capability | Coverage | Priority | Current FFI surface and constraints | Evidence and FFI work if pursued |
|---|---|---:|---|---|
| Connector negotiation and active-session processing | Exposed | Baseline | The caller owns transport I/O and drives connector/activation sequences and session PDUs. | `ffi/src/connector/mod.rs`; `ffi/src/session/mod.rs`; the .NET transport driver is in `ffi/dotnet/Devolutions.IronRdp/src/Connection.cs`. |
| Common connection configuration | Limited | High | The builder covers credentials, desktop size, keyboard, bitmap, and several client flags. Monitor layout and physical display data are fixed to `None`; dynamic graphics and RemoteApp settings are fixed off. Native audio attachment enables capture when requested. | `ffi/src/connector/config.rs`; `ffi/src/audio.rs`; the Rust client has monitor-layout and RemoteApp builder methods (`crates/ironrdp-client/src/config.rs`). Expose additional settings only where needed, while preserving valid combinations. |
| Desktop input, framebuffer, display resize, touch, and pen | Exposed | Baseline | The low-level API processes input and image updates, provides Display Control resize, and encodes RDPEI touch and pen frames. | `ffi/src/session/mod.rs`; `ffi/src/input.rs`. |
| Clipboard | Limited | Medium | CLIPRDR messages and custom backend factories are exposed. The provided OS backend is Windows-only; file transfer is an opt-in feature of that backend. | `ffi/src/clipboard/mod.rs`; `ffi/src/clipboard/windows.rs`; Rust client feature and config: `crates/ironrdp-client/Cargo.toml`, `crates/ironrdp-client/src/config.rs`; add or integrate platform backends only where required. |
| Graphics pipeline and video remoting | Disabled | High | The FFI config disables dynamic graphics protocol support; it does not expose EGFX or H.264 decoder configuration. The Rust client supports EGFX and optional video remoting, subject to decoder and platform availability. | `ffi/src/connector/config.rs`; `crates/ironrdp-client/README.md`; add EGFX channel setup, decoder selection, and a managed way to deliver decoded updates if pursued. |
| RemoteApp / RAIL | Disabled | High | RemoteApp mode is false and RAIL support is empty. Windowing orders can be read, but the FFI notes that window-surface pixels are not exposed. | `ffi/src/connector/config.rs`; `ffi/src/session/mod.rs`; expose RAIL configuration and input plus window-surface payloads if required. |
| Audio playback and capture | Limited | Medium | Native playback and opt-in microphone capture use Rust device backends, while the caller pumps outbound frames. Device failures are logged through Rust tracing. Android native audio is unsupported; on Linux, the FFI library requires `libasound` even when audio is disabled. The older no-op playback setup remains available. | `ffi/src/audio.rs`; `ffi/src/connector/mod.rs`; `ffi/src/session/mod.rs`; see the [audio usage guide](README.md#native-audio). Device availability, permissions, and codec support depend on the platform. |
| RDPDR and device redirection | Limited | Medium | The exposed RDPDR setup announces a smartcard; Windows constructs its backend with no drives, and other platforms use a no-op backend. | `ffi/src/connector/mod.rs`; Rust client config: `crates/ironrdp-client/src/config.rs`; expose device/backend configuration and the corresponding asynchronous completion paths for any additional device types. |
| Client-owned transports and advanced authentication | Absent | Medium | The FFI exposes connector primitives rather than the Rust client's managed gateway, reliable UDP, Remote Credential Guard, or Entra/AVD authentication flows. Consumers can supply their own transport around the low-level connector, but that is not equivalent to these client features. | `ffi/Cargo.toml`; `crates/ironrdp-client/Cargo.toml`; integrating any one of these would need explicit configuration and protocol-state APIs, not just another connector flag. |
| Other optional client channels | Limited | Medium | Generic direct-DVC events and payloads are exposed, but the FFI has no channel-specific setup for camera, USB, PnP, location, Windows 365, text input, or MultiPoint. Rust support is feature-gated and includes platform-specific limits. | `ffi/src/dvc/direct_channel.rs`; `crates/ironrdp-client/Cargo.toml`; `crates/ironrdp-client/README.md`; each channel needs its own protocol handling and platform/backend policy. |

## Scope

This assessment keeps the existing low-level, .NET-oriented FFI approach.
The Rust `ironrdp-client::RdpClient` async runtime and its transport driver are not wrapped; the FFI consumer supplies transport I/O and advances the exposed state machines.
That is a boundary choice, not a finding that a higher-level client wrapper is required.

Server and acceptor APIs are outside this client-oriented matrix.
The Rust workspace exposes them separately (`crates/ironrdp/Cargo.toml`), while the FFI docs identify .NET as the supported target and describe client connection/session use (`ffi/README.md`).

The Rust client features cited here are optional or platform-dependent, so their presence in the crate does not mean they are enabled in every build or work on every target.
“High” and “Medium” indicate the breadth of an FFI surface addition, not known customer demand.
