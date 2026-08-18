# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.18.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.17.0...ironrdp-v0.18.0)] - 2026-08-18

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
