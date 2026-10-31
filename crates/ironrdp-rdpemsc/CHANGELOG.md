# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.0.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpemsc-v0.0.0)] - 2026-10-31

### <!-- 1 -->Features

- Add the MS-RDPEMSC mouse cursor dynamic channel ([#2321](https://github.com/Devolutions/IronRDP/issues/2321)) ([bbd1b879ce](https://github.com/Devolutions/IronRDP/commit/bbd1b879cead8fbb8333f94fdebe337d0c0105e5)) 

  Implements [MS-RDPEMSC] (the `Microsoft::Windows::RDS::MouseCursor`
  dynamic channel) and connects it to the client's existing pointer
  handling.
  
  The channel is **off by default** because it hasn't been run against a
  real Windows or AVD server yet. With it off, the client declines the
  channel as before, and pointer updates stay on the fast path. To turn it
  on, call `ConfigBuilder::with_mouse_cursor_channel(true)`:
  
  ```rust
  let builder = builder.with_mouse_cursor_channel(true);
  ```
  
  You can also set `ChannelConfig::mouse_cursor = true` directly.
  
  ## New crate: `ironrdp-rdpemsc`
  
  - **PDUs** (`no_std`): encode and decode for the capability advertise,
  capability confirm and pointer update messages.
  - TS_POINTERATTRIBUTE and TS_LARGEPOINTERATTRIBUTE have the same byte
  layout as in MS-RDPBCGR, so the update payloads reuse the codecs in
  `ironrdp_pdu::pointer`.
  - `PointerUpdate::as_pointer_update_data()` converts an update into the
  existing `PointerUpdateData`.
  - Encoding a version 1 capability set that carries data returns an
  error, matching what the decoder accepts.
  - **`MouseCursorClient`**: sends the version 1 capabilities when the
  channel starts, and becomes ready only when the server confirms version
  1.
  - Pointer updates are queued for the caller, up to 64 of them. Updates
  that arrive before the confirmation are queued too.
  - When the queue is full, it first drops updates that a later one
  replaces: older positions, older visible-pointer changes, and older
  shapes for the same cache slot. Applying what's left gives the same
  position, visible pointer and pointer cache. The oldest update is
  dropped only if the queue still holds a shape for 64 different cache
  slots; the client advertises 32.
    - Unexpected or malformed messages are logged and ignored.
  - **`MouseCursorServer`**: when the client advertises version 1, it
  replies with a version 1 confirmation and becomes ready. It is not wired
  into `ironrdp-server` yet.
  
  ## Wiring
  
  - `ironrdp-session`: new `ActiveStage::process_pointer_update(image,
  PointerUpdateData)`. It uses the same pointer cache, server-pointer
  setting and software rendering as fast-path pointer updates. The
  update-to-output loop in `process` moved into a private helper that both
  paths share. The helper also widens the first graphics update to a full
  refresh after a graphics reset.
  - `ironrdp-client`:
  - New `ChannelConfig::mouse_cursor` field, `false` by default, set with
  `with_mouse_cursor_channel`.
  - When it is on, `rdp.rs` registers `MouseCursorClient` on the DRDYNVC
  client.
  - The new `mouse_cursor::present` applies the queued updates after each
  session loop iteration, the same way `video::present` does. When the
  channel isn't registered, it does nothing.
  - Also updated: fuzzing oracles (decode and round-trip), the
  `ARCHITECTURE.md` entry, and both lockfiles.
  
  ## Tests (`ironrdp-testsuite-core/tests/rdpemsc`)
  
  - Spec examples from §4.1.1, §4.1.2, §4.2.1 and §4.2.2.
  - Large pointer layout (§2.2.2.6), the optional pad byte, the hidden,
  default and cached updates, and capability size and signature checks,
  including the encode-side check.
  - Client and server behaviour:
  - the handshake, including that a confirmation of capabilities the
  client didn't advertise leaves it not ready;
    - ignoring unexpected messages, and reset on close.
  - The queue:
    - a cached update still finds a shape sent 64 updates earlier;
  - the oldest update is dropped only when every queued update is a shape
  for a different cache slot;
  - a property test checks that the queued updates leave the same pointer
  state as all received updates.
  - Through `ActiveStage`: a cached pointer gives the same image as the
  shape that filled its slot, and position updates are applied.
  - A proptest round trip that also checks `encode` writes exactly
  `size()` bytes.
  
  ## Not tested
  
  - I haven't run it against a real Windows or AVD server, which is why it
  is off by default.
  - `mouse_cursor::present` in the client has no test of its own. The
  pieces on either side (client queueing and
  `ActiveStage::process_pointer_update`) are tested.
  - Locally, `cargo xtask ci` passed fmt, typos, tests, clippy, the
  feature matrix, dependencies, captures and the WASM build. It stopped at
  `wasm2wat`, which isn't installed. The fuzz step (needs `cargo-fuzz` on
  nightly) and the web step (npm) didn't run. `cargo xtask check locks`
  passes.
  - The Soft-Sync comment in `crates/ironrdp-dvc/src/client.rs` still
  names MouseCursor as a declined channel. I left it alone because the
  MS-RDPECI change touches the same line.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------


