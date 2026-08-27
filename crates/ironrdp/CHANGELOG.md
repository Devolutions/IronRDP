# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.18.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.17.0...ironrdp-v0.18.0)] - 2026-08-27

### <!-- 0 -->Security

- Connect to Windows Sandbox named pipes ([#1580](https://github.com/Devolutions/IronRDP/issues/1580)) ([39b020343d](https://github.com/Devolutions/IronRDP/commit/39b020343d962962bfbefc89939be64d5c716196)) 

  Windows Sandbox's default attach path is a local named pipe carrying
  plain TPKT/X.224 with PROTOCOL_RDP and ENCRYPTION_LEVEL_NONE, not
  TCP:3389 or VMConnect. Allow the connector and client to complete that
  sequence only via an explicit opt-in (`enable_standard_rdp_security`;
  NamedPipe enables it), and teach ironrdp-agent to resolve pipe path and
  guest credentials from WindowsSandboxServer after `wsb start`.
  
  Adds Transport::NamedPipe, ironrdp_named_pipe/ironrdp_sandbox_id
  properties, sandbox list/config/stop CLI helpers via an in-process
  h2/gRPC client on the per-user `\\.\pipe\wsandbox\{guid}` pipe (no .NET
  helper), and connect --sandbox-id / --sandbox-pipe. Sandbox-derived
  properties are the merge base; explicit .rdp/--prop/flags override them
  while NamedPipe TLS/CredSSP stay forced off. Local :2179+PCB remains
  unsupported.

### <!-- 1 -->Features

- Expose generic session configuration and lifecycle APIs ([#1522](https://github.com/Devolutions/IronRDP/issues/1522)) ([57b1366650](https://github.com/Devolutions/IronRDP/commit/57b13666506dc40c15b4c4702d35150beee99133)) 

  ## Summary
  - expose generic client configuration for connection metadata,
  compression, shell/work directory, audio, and runtime static-channel
  factories
  - add bounded input delivery with independent close cancellation, host
  clipboard plumbing, lifecycle events, and Display Control resize
  readiness/fallback handling
  - update agent, viewer, web, FFI, examples, and tests for the generic
  APIs
  
  ## Stack dependencies
  This PR is stacked on `copilot/tls-validation-policy` (`b2bbcece`),
  which already includes the merged runtime static-channel support from
  `master`. It intentionally contains no TLS implementation/policy,
  ActiveX/COM, SVC implementation, decompression, or bitmap-recovery
  changes.
  
  ## Validation
  - `cargo fmt --check --all`
  - `cargo xtask check tests --no-run -v`
  - `cargo xtask check lints -v`
  - `cargo test -p ironrdp-client --lib --features rustls`
  - `cargo check -p ironrdp-agent -p ironrdp-viewer -p ironrdp-web -p ffi`
  
  ---------

- Hyper-V vmconnect support ([#1503](https://github.com/Devolutions/IronRDP/issues/1503)) ([a7cc067d50](https://github.com/Devolutions/IronRDP/commit/a7cc067d5069cbbcb13bae3e0561c0611da3bcf6)) 

  Adds Hyper-V VMConnect's direct ordering: PCB → TLS → CredSSP → X.224.
  
  Enhanced Session is the default (`GUID;EnhancedMode=1`), with
  `--vmconnect-basic` for the synthetic console. Kept this separate in
  `ironrdp-vmconnect`; no SPN changes.
  
  Tested against the nested Hyper-V lab:
  - Enhanced: `HYBRID_EX`, rendered 1280×720
  - Basic: `HYBRID`, rendered 1280×720
  - `cargo xtask check fmt/lints/tests -v`
  
  ---------

- Add RemoteApp channel support ([#1637](https://github.com/Devolutions/IronRDP/issues/1637)) ([ab48c6cb8c](https://github.com/Devolutions/IronRDP/commit/ab48c6cb8c017504f8a92799aeb91b821c50a13a)) 

  Configure and negotiate RAIL connections, then route its static channel
  through the portable client with bounded request queues and server
  control events.

- Add Input DVC and ActiveX touch ([#1647](https://github.com/Devolutions/IronRDP/issues/1647)) ([a912e19bd2](https://github.com/Devolutions/IronRDP/commit/a912e19bd2bb31f403fd7c35c8efd729a5ab5f6f)) 

  Implement MS-RDPEI for multi-touch over the dynamic virtual channel
  Microsoft::Windows::RDS::Input, and wire Windows pointer messages in
  ActiveX through session encode helpers.
  
  Introduce ironrdp-rdpei PDUs and processors, register the channel from
  the client, encode touch frames from ActiveX WM_POINTER*, and cover the
  protocol with unit and integration tests.

- Wire MS-RDPEAI capture into Windows client and ActiveX ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d)) 

  Add the client MS-RDPEAI capture path on top of hardened RDPSND
  playback: connector CFG + static channel wiring, CPAL PCM capture
  backend, ironrdp-client --audio-capture, and ActiveX
  AudioCaptureRedirectionMode.
  
  PCM capture only accepts encode formats that match the Open capture
  stream, rejects non-16-bit capture (Data PDU size contract), and gates
  the capture backend behind ironrdp-rdpsnd-native/capture.
  
  Depends on #1648 (playback).

- Negotiate monitor topology ([#1675](https://github.com/Devolutions/IronRDP/issues/1675)) ([063efcdc30](https://github.com/Devolutions/IronRDP/commit/063efcdc3088d8f44e423cc322077d40bf9aadf2)) 

  Negotiate the client monitor layout from UseMultimon and expose the
  confirmed remote topology through the ActiveX compatibility interface.
  
  Advertise Monitor Layout PDU support whenever Extended Client Data is
  negotiated, and forward layouts from activation, active sessions, and
  reactivation so advertised support does not terminate sessions.
  
  Keep fallback reporting truthful when servers do not honor the request,
  while preserving single-monitor resize behavior and blocking
  multi-monitor resizing.
  
  Do not send Client Monitor Extended Data; per-monitor DPI and
  orientation remain unavailable.

- [**breaking**] Introduce typed ServerError on the public API boundary ([#1242](https://github.com/Devolutions/IronRDP/issues/1242)) ([b5b9558eae](https://github.com/Devolutions/IronRDP/commit/b5b9558eae164a3500ace069b4db12be25eba7bd)) 

  ## Summary
  
  First in a staged migration toward a typed public error story for
  `ironrdp-server`, addressing #1209. Introduces:
  
  ```rust
  pub type ServerError = ironrdp_error::Error<ServerErrorKind>;
  pub type ServerResult<T> = Result<T, ServerError>;
  ```
  
  `ServerErrorKind` is a `#[non_exhaustive]` enum with concretely typed
  variants (`Encode`, `Io`, `Channel`, `Unsupported`, `Reason`, `Custom`).
  Sources are attached through `ironrdp_error::Error::with_source` rather
  than embedded as `Box<dyn Error>` in variant data. This mirrors the
  shape of `ConnectorErrorKind` in `ironrdp-connector`, so the
  connection-management layer stays internally consistent.
  
  ## Why this shape
  
  The audit before drafting found that `ironrdp-connector`,
  `ironrdp-session`, `ironrdp-pdu`, `ironrdp-mstsgu`, and `ironrdp-core`
  (`EncodeError`/`DecodeError`) all use the same
  `ironrdp_error::Error<Kind>` pattern. The `thiserror`-based bare enums
  elsewhere in the workspace (`GccError`, `BulkError`, etc.) are
  leaf-layer errors at the PDU/codec level; the connection-management
  layer sister crate to `ironrdp-server` is the right template.
  
  ## Scope of this PR
  
  Public function signatures only:
  
  - `RdpServer::run` → `ServerResult<()>`
  - `RdpServer::run_connection<S>` → `ServerResult<()>`
  - `RdpServer::run_connection_with<S>` → `ServerResult<()>`
  - `TlsIdentityCtx::init_from_paths` → `ServerResult<Self>`
  - `TlsIdentityCtx::make_acceptor` → `ServerResult<TlsAcceptor>`
  - `EchoServerHandle::send_request` → `ServerResult<()>`
  
  Internal call sites continue to use `anyhow::Result`. A private
  `from_anyhow_with_context` helper bridges at the public boundary,
  tagging each call site with the operation that failed. The
  `ConnectionHandler::on_disconnected(error: Option<&anyhow::Error>)`
  parameter is unchanged in this PR.
  
  `run_connection_with` keeps its anyhow body in a private
  `run_connection_with_inner`, which the accept loop calls directly so
  `on_disconnected` still receives an `anyhow::Error`. That helper is
  transitional and its removal is inside this stack rather than deferred
  indefinitely: #1244 converts `on_disconnected` to `ServerError` and
  deletes it, with the accept loop calling `run_connection` from there.
  `TlsIdentityCtx::init_from_paths` and `make_acceptor` use the same
  wrapper-plus-private-body shape, matching the `run` / `run_inner` pair
  this PR already introduces.
  
  ## Companion ext traits
  
  ```rust
  pub trait ServerErrorExt {
      fn encode(error: EncodeError) -> Self;
      fn io(context: &'static str, error: io::Error) -> Self;
      fn channel(context: &'static str) -> Self;
      fn unsupported(context: &'static str) -> Self;
      fn reason(context: &'static str, reason: impl Into<String>) -> Self;
      fn custom<E>(context: &'static str, error: E) -> Self
      where E: core::error::Error + Sync + Send + 'static;
  }
  pub trait ServerResultExt {
      fn with_context(self, context: &'static str) -> Self;
  }
  ```
  
  Mirrors `ConnectorErrorExt` / `ConnectorResultExt`, trimmed to the
  constructors and the one result-side helper that have call sites in this
  stack (`decode` and `with_source` did not, so neither shipped).
  
  ## Stack ordering
  
  This is step 1 of 4. The full chain (must merge in PR-number order):
  
  | Step | PR | Scope |
  |------|-----|-------|
  | **1** | **this PR** | Add `ServerError` / `ServerErrorKind` / ext
  traits, convert 5 public functions, internal anyhow stays via private
  bridge |
  | 2 | #1243 | Convert encoder/helper/echo internals (anyhow → typed) |
  | 3 | #1244 | Convert server.rs internals + `on_disconnected` parameter
  (breaking) |
  | 4 | #1245 | Convert display traits, drop anyhow dep, finish (breaking)
  |
  
  All four close #1209 together. Each step is independently reviewable but
  is rebased onto its predecessor as a Stacked branch; please merge in the
  order above so the rebases land cleanly.
  
  ## Breaking change
  
  Marked with `!` in the conventional commit. Consumers of the five listed
  public functions need to update their `Result` types. Pre-1.0, and per
  @elmarco's note on #1209: "I am ok with breaking API at this point :)".
  
  ## Test plan
  
  - `cargo xtask check fmt -v` clean
  - `cargo xtask check lints -v` clean (workspace, all-targets, with
  helper + __bench features)
  - `cargo xtask check tests -v` passes
  - `cargo build --workspace --all-targets` clean (one example consumer in
  `crates/ironrdp/examples/server.rs` updated to convert at its own
  boundary)
  
  Closes part of #1209.

- [**breaking**] Convert display traits to ServerResult, drop anyhow dep ([#1245](https://github.com/Devolutions/IronRDP/issues/1245)) ([9abd6eb482](https://github.com/Devolutions/IronRDP/commit/9abd6eb482d8e7bcc64d578fc0a97bb12b6c82d9)) 

  ## Summary
  
  Final step of the staged migration started in #1242 and continued in
  #1243 / #1244. Completes the typed error story for `ironrdp-server` by
  converting the last consumer-facing trait surface and dropping the
  `anyhow` dependency entirely. Closes #1209.
  
  ## Public API changes
  
  ```diff
   pub trait RdpServerDisplayUpdates {
  -    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>>;
  +    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>>;
   }
  
   pub trait RdpServerDisplay: Send {
  -    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>>;
  +    async fn updates(&mut self) -> ServerResult<Box<dyn RdpServerDisplayUpdates>>;
   }
  ```
  
  These are **breaking changes** for handler implementations of the two
  display traits.
  
  ## Internal changes
  
  - `from_anyhow` private bridge and `AnyhowError` wrapper struct removed
  from `error.rs`.
  - `anyhow` dependency removed from `ironrdp-server/Cargo.toml`.
  - `builder.rs` `NoopDisplayUpdates` / `NoopDisplay` impls and the
  docstring examples in `display.rs` and `README.md` updated to match the
  new trait shapes.
  - `crates/ironrdp/examples/server.rs` and
  `crates/ironrdp-testsuite-extra/tests/main.rs` updated to return
  `ServerResult` from their `RdpServerDisplay/Updates` impls.
  - `benches/src/perfenc.rs` updated to construct `ServerError` variants
  instead of `anyhow::Error` and converts at its own `anyhow::Result` main
  boundary via `.map_err(|e| anyhow::anyhow!(e))`.
  - `urbdrc.rs`, the `usb`-gated USB device facade, converted from
  `anyhow` to `ServerResult`/`ServerError` (69 sites). This file predates
  the typed error migration and was never touched by it, so removing the
  `anyhow` dependency broke it under the `usb` feature. Typed external
  errors from `ironrdp-usb`/`ironrdp-rdpeusb` are wrapped with
  `ServerError::custom`; hand-rolled invariant checks (former
  `bail!`/`ensure!`) use `ServerError::reason`.
  
  ## After this PR
  
  `ironrdp-server` has **no anyhow dependency** and the public surface is
  fully typed against `ServerError`, including the `usb`-gated facade. The
  full chain:
  
  | Step | PR | Scope |
  |------|-----|-------|
  | 1 | #1242 | Add `ServerError` / `ServerErrorKind` / ext traits,
  convert 5 public functions, internal anyhow stays via private bridge |
  | 2 | #1243 | Convert encoder/helper/echo internals |
  | 3 | #1244 | Convert server.rs internals + `on_disconnected` parameter
  |
  | **4** | **this PR** | Convert display traits and the usb facade, drop
  anyhow dep, finish |
  
  ## Stacking note
  
  Stacked on `feat/server-typed-error-server` ([#1244](https://github.com/Devolutions/IronRDP/issues/1244)). Rebase on landing
  in PR-number order.
  
  ## Test plan
  
  - `cargo xtask check fmt -v` clean
  - `cargo xtask check lints -v` clean (workspace, all-targets, with
  helper + __bench features)
  - `cargo xtask check tests -v` passes (including doctests)
  - `cargo xtask check features --case workspace/powerset-runtime` clean
  (44/44, covers `ironrdp-server` across the full feature powerset
  including `usb`)
  - `cargo build --workspace --all-targets` clean

- Add location redirection ([#1778](https://github.com/Devolutions/IronRDP/issues/1778)) ([1cee7a8613](https://github.com/Devolutions/IronRDP/commit/1cee7a86135a0556c01965d0406233bd7df367a9)) 

  Implement MS-RDPEL v1 codecs and the location DVC state machine, then
  route the ActiveX methods through the bounded client input queue.
  
  Preserve mstsc-compatible validation and altitude caching while
  surfacing inactive sessions, channel readiness, queue pressure, and
  encoding failures. Coordinates are caller-supplied only and are never
  logged or persisted.

### <!-- 4 -->Bug Fixes

- [**breaking**] Always own a bulk decompressor for FastPath updates ([#1255](https://github.com/Devolutions/IronRDP/issues/1255)) ([0dc0194418](https://github.com/Devolutions/IronRDP/commit/0dc0194418375d504a8041b75ba250dc8eeb21ad)) 

  ## Summary
  
  - A compressed FastPath update is dropped whenever the client did not
  negotiate compression, because the decompressor is only built when a
  compression type was negotiated. Servers send compressed updates
  regardless, for example on a full-frame redraw after a resize, and the
  session then fails. Closes #1193.
  - The negotiated type is the wrong thing to condition on. It describes
  what the client would send, and nothing in `ironrdp-session`,
  `ironrdp-client`, `ironrdp-web` or the FFI ever compresses outbound. On
  the receive path `BulkCompressor` holds a context per algorithm and
  `decompress` selects one per update from the packet's own type bits, so
  a decompressor built with any type decodes all of them.
  - The `Processor` now owns the decompressor and builds it on the first
  update that needs one. `ProcessorBuilder` has no corresponding field, so
  there is no `None` a consumer can pass and no path that drops a
  compressed update.
  - On demand rather than at construction because `ironrdp-web` hardcodes
  `compression_type: None` in `build_config` and so never negotiates
  compression. Constructing eagerly would charge every web session for a
  full set of algorithm contexts, and the two XCRUSH history buffers alone
  are 2 MB each, for a decompressor most of those sessions never use. That
  consumer is also the one most exposed to this bug, for the same reason.
  - `BulkCompressor::new` is now infallible. Its only failure path was a
  self-check over NCRUSH's static Huffman tables, a compile-time
  invariant, now a `debug_assert`.
  
  ## Relationship to #1474
  
  #1474 is kept, not reverted. `ActiveStage::reactivate` is adopted as the
  reactivation entry point at all four call sites it introduced: native
  client, web, FFI and the e2e test.
  
  What this PR removes is the `compression_type` retained on `ActiveStage`
  and the `make_bulk_decompressor` helper, because an on-demand
  decompressor makes both unnecessary. `reactivate` keeps its behaviour
  and loses only the compression plumbing.
  
  #1474 closed the reactivation instance of #1193, where a rebuild passed
  `None` and silently disabled decompression for the rest of the session.
  The general case is still open on master: when compression was never
  negotiated the retained type is `None`, `make_bulk_decompressor` returns
  `None`, and every compressed update takes the drop path in
  `fast_path.rs` for the lifetime of the session. Conditioning on the
  negotiated type gates the ability to receive on what was negotiated to
  send, and nothing sends.
  
  The evidence that removing the field is safe is #1474's own test.
  `test_reactivation_processes_compressed_fastpath_updates` passes
  unchanged with `compression_type` gone from the builder: the rebuilt
  processor decompresses because every processor can, not because a type
  was carried across the rebuild.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  The gated regression test is
  `testsuite-core/tests/session/fast_path.rs`, which renders the same
  bitmap update plain and bulk-compressed through fresh processors and
  asserts identical framebuffers. #1474's
  `test_reactivation_processes_compressed_fastpath_updates` in
  `testsuite-extra` passes unchanged.
  
  There is also an inline test in `fast_path.rs` pinning the allocation
  invariant, that no contexts are built until an update needs them. Note
  that `ironrdp-session` sets `[lib] test = false`, so inline tests in
  this crate are not run by `cargo test --workspace`; it runs under `cargo
  test -p ironrdp-session --lib`.
  
  ## Notes
  
  - This addresses the four points from the 2026-06-24 review. Point 4,
  that the `Option` is misleading, is the shape of this change: it is gone
  from the public API, and the private one that remains carries no
  implication that a consumer could choose not to decompress. Point 1,
  whether a cold `Rdp61` context decodes `RDP40` and `RDP50` updates
  correctly, is a non-issue: `decompress` selects the algorithm per update
  through `CompressionType::from_flags` against per-algorithm receive
  contexts, so the construction-time type never constrains the receive
  path. Point 3, silent degradation if the constructor fails, is removed
  by making `new` infallible. Point 2 is the tests above.
  - Breaking across two crates, hence the `fix(bulk,session)!` scope:
  `ProcessorBuilder` loses `bulk_decompressor`, `ActiveStageBuilder` loses
  `compression_type`, and `ironrdp_bulk::BulkCompressor::new` returns
  `Self`.
  - Incidental: `ironrdp-session` no longer exposes any `ironrdp_bulk`
  type in its public API, so that dependency's lack of a `# public` marker
  in `Cargo.toml` is now correct.

- Share bulk decompression across output paths ([#1518](https://github.com/Devolutions/IronRDP/issues/1518)) ([6151e21bf5](https://github.com/Devolutions/IronRDP/commit/6151e21bf58b7297e9b4abc2167aa36fc2ba77e4)) 

  Bulk compression state is stream-wide, but Fast-Path and slow-path
  outputs previously used separate or missing decompression paths. This
  could corrupt history-dependent server updates or leave negotiated
  slow-path compression undecodable.
  
  This change owns the negotiated bulk decompressor in `ActiveStage` and
  passes it to both X.224 and Fast-Path processing. It retains Share Data
  compression metadata through the PDU context, resets decompression
  history on reactivation, and initializes consumers from the connection's
  negotiated compression type.
  
  Fast-Path now decompresses each fragment before reassembly so
  compression flags apply at packet boundaries. Failures expose bounded
  protocol metadata without retaining remote payloads or decoder details.
  
  Tests cover Share Data metadata propagation, slow-path decompression
  behavior, fragmented Fast-Path reassembly and bounded errors, and
  compressed Fast-Path updates after reactivation.
  
  ---------

- [**breaking**] Accept the full documented range of Client Core Data keyboardType values ([#1689](https://github.com/Devolutions/IronRDP/issues/1689)) ([c74e7c5c94](https://github.com/Devolutions/IronRDP/commit/c74e7c5c9431c87e57a1550b037867d235c1d362)) 

  ## Summary
  
  KeyboardType (TS_UD_CS_CORE's keyboardType field, MS-RDPBCGR 2.2.1.3.2)
  was a closed enum covering discriminants 1 through 7, transcribed from
  the field's other documentation in 2.2.7.1.6 (TS_INPUT_CAPABILITYSET),
  which omits value 8 (Korean keyboard). The table this type actually
  decodes against, 2.2.1.3.2, documents 1 through 8. Any client outside
  the narrower range, including every genuine Korean keyboard, had its
  whole Client Core Data parse hard rejected before the connection
  started.
  
  Converted KeyboardType to a repr(transparent) struct KeyboardType(pub
  u32) with named constants for the eight documented values, mirroring
  RdpVersion one field above it in the same struct, and made decode
  infallible at all three sites that previously disagreed with each other:
  gcc::core_data::client hard rejected, rdp::capability_sets::input
  silently dropped to None, and ironrdp-activex's COM setter returned
  E_INVALIDARG.
  
  FreeRDP hit the identical gap in its own documentation-only enum until a
  two-line fix (FreeRDP#11035). Neither FreeRDP nor xrdp gate connection
  admission on this field at all; both decode it as a raw integer.
  Windows' own GetKeyboardType additionally documents 0x51 for generic HID
  keyboards, which a narrower fix adding only Korean would not survive.
  
  Marked as breaking since KeyboardType's public shape changes for any
  downstream consumer of ironrdp-pdu outside this workspace. Note that
  cargo-semver-checks in this repo's PR automation only runs against the
  facade ironrdp crate, which will not see a break confined to
  ironrdp-pdu's own API, so the automated breaking-change label may not
  apply here even though this is a real one.
  
  ## Validation
  
  cargo xtask check fmt/lints/tests/typos/locks all pass. Extended
  ironrdp-activex's existing inline COM boundary test to cover value 8
  round tripping and a negative value still being rejected.

- [**breaking**] Drop the tokio and tokio_rustls re-exports ([#1796](https://github.com/Devolutions/IronRDP/issues/1796)) ([cac7bd6f37](https://github.com/Devolutions/IronRDP/commit/cac7bd6f37120711570b0156dd87c8ce6145fad6)) 

  ## Summary
  
  `ironrdp-server` re-exported both `tokio` and `tokio_rustls` wholesale
  so consumers did not need their own `tokio` dependency. This pins every
  consumer's `tokio` version to whatever `ironrdp-server` picks, with no
  way to upgrade independently. Drops the re-export.
  
  ```diff
  -pub use {tokio, tokio_rustls};
  ```
  
  ## Public API changes
  
  - `ironrdp_server::tokio` and `ironrdp_server::tokio_rustls` no longer
  exist. Consumers using either path need their own `tokio` and
  `tokio-rustls` dependencies.
  - `tokio-rustls` stays marked `# public` in `Cargo.toml`: `TlsAcceptor`
  genuinely appears in public signatures (`with_tls`, `with_hybrid`,
  `TransportTls::Tls`). `tokio`'s `# public` marker is dropped since no
  public signature returns or takes a bare `tokio` type; it was only
  public via the re-export.
  
  ## Internal changes
  
  - The one workspace consumer, `crates/ironrdp/examples/server.rs`, now
  imports `tokio` directly instead of through the facade.
  - `tokio` added as a direct dev-dependency of the `ironrdp` facade
  crate, and to its existing `#[cfg(test)] use { ... as _ }`
  unused-dependency silencer, the same pattern already used there for
  every other example-only dependency.
  
  ## Test plan
  
  `cargo xtask check fmt/lints/tests/typos/locks` clean. `cargo xtask
  check features --case workspace/powerset-runtime` clean (44/44, covers
  `ironrdp-server`).

### <!-- 7 -->Build

- Bump the crypto group across 1 directory with 3 updates ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b)) 



## [[0.17.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.16.0...ironrdp-v0.17.0)] - 2026-07-10

### <!-- 0 -->Security

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 1 -->Features

- Gate native backends behind Cargo features ([#1338](https://github.com/Devolutions/IronRDP/issues/1338)) ([f7e6106e0f](https://github.com/Devolutions/IronRDP/commit/f7e6106e0f293c1e0f8129be82aa2d86737ba92a)) 

  - Added:    client, client-all, client-sound, client-clipboard,
              client-rdpdr, client-smartcard, client-gateway,
              client-dvc-pipe-proxy, client-dvc-com-plugin, and
              top-level rustls / native-tls (forwarded to ironrdp-client)
  - Modified: qoi, qoiz now also gate ironrdp-client's codec

- [**breaking**] Misuse-resistant format negotiation for RdpsndServerHandler ([#1359](https://github.com/Devolutions/IronRDP/issues/1359)) ([2d3bdef1a7](https://github.com/Devolutions/IronRDP/commit/2d3bdef1a7167d2acdc478a92917cbb2f018960b)) 

  Move the negotiation into the crate and split selection from lifecycle:
  
  ```rust
  fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat>;
  fn start(&mut self, format: &NegotiatedFormat);
  ```

### <!-- 4 -->Bug Fixes

- [**breaking**] Remove ironrdp-connector dependency ([#1435](https://github.com/Devolutions/IronRDP/issues/1435)) ([c6a0286dcb](https://github.com/Devolutions/IronRDP/commit/c6a0286dcb49d9ac54c65c4f9325b41e05d541b8)) 

  Removes the last ironrdp-connector coupling from ironrdp-session by
  turning Deactivate-All handling into a bare signal and shifting ownership
  of the Deactivation-Reactivation activation sequence back to each consumer.
  It introduces a ConnectionActivationFactory (fresh sequence per reactivation)
  and an ActiveStageBuilder so session construction no longer depends on
  ConnectionResult.



## [[0.16.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.15.0...ironrdp-v0.16.0)] - 2026-06-05

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-displaycontrol`, `ironrdp-dvc`, `ironrdp-echo`, `ironrdp-server`, and `ironrdp-session` public dependencies



## [[0.15.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.14.0...ironrdp-v0.15.0)] - 2026-05-27

### Build

- Update dependencies

## [[0.14.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.13.0...ironrdp-v0.14.0)] - 2025-12-18

### <!-- 7 -->Build

- Bump picky and sspi ([#1028](https://github.com/Devolutions/IronRDP/issues/1028)) ([5bd319126d](https://github.com/Devolutions/IronRDP/commit/5bd319126d32fbd8e505508e27ab2b1a18a83d04)) 

  This fixes build issues with some dependencies.

## [[0.13.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.12.0...ironrdp-v0.13.0)] - 2025-09-24

### Build

- Update dependencies

## [[0.12.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.11.0...ironrdp-v0.12.0)] - 2025-08-29

### Build

- Update dependencies

## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.10.0...ironrdp-v0.11.0)] - 2025-07-08

### Build

- Update dependencies

## [[0.9.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.9.0...ironrdp-v0.9.1)] - 2025-03-13

### <!-- 6 -->Documentation

- Fix documentation build (#700) ([0705840aa5](https://github.com/Devolutions/IronRDP/commit/0705840aa51bc920e76f0cf1fce06b29733c6e2d)) 

## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.8.0...ironrdp-v0.9.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.4...ironrdp-v0.8.0)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.7.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.3...ironrdp-v0.7.4)] - 2025-01-28

### Build

- Update dependencies

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

- Extend server example to demonstrate Opus audio codec support (#643) ([fa353765af](https://github.com/Devolutions/IronRDP/commit/fa353765af016734c07e31fff44d19dabfdd4199)) 


## [[0.7.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.2...ironrdp-v0.7.3)] - 2024-12-16

### <!-- 6 -->Documentation

- Inline documentation for re-exported items (#619) ([cff5c1a59c](https://github.com/Devolutions/IronRDP/commit/cff5c1a59cdc2da73cabcb675fcf2d85dc81fd68)) 


## [[0.7.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.1...ironrdp-v0.7.2)] - 2024-12-15

### <!-- 6 -->Documentation

- Fix server example ([#616](https://github.com/Devolutions/IronRDP/pull/616)) ([02c6fd5dfe](https://github.com/Devolutions/IronRDP/commit/02c6fd5dfe142b7cc6f15cb17292504657818498)) 

  The rt-multi-thread feature of tokio is not enabled when compiling the
  example alone (without feature unification from other crates of the
  workspace).


## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.0...ironrdp-v0.7.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
