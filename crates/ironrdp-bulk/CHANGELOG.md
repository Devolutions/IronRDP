# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-bulk-v0.1.1...ironrdp-bulk-v0.2.0)] - 2026-08-08

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



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-bulk-v0.1.0...ironrdp-bulk-v0.1.1)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Gate alloc-dependent modules behind the alloc feature ([#1279](https://github.com/Devolutions/IronRDP/issues/1279)) ([18a430a51a](https://github.com/Devolutions/IronRDP/commit/18a430a51aca07aa45db4642df5c932ef65d2016)) 

- Off-by-one in forward match loop causes panic during compression ([#1293](https://github.com/Devolutions/IronRDP/issues/1293)) ([0dd7c94ba2](https://github.com/Devolutions/IronRDP/commit/0dd7c94ba22e9bd11b4ea36fd03af3bfcccecab8)) 

### <!-- 7 -->Build

- Bump criterion from 0.5.1 to 0.8.1 ([#1184](https://github.com/Devolutions/IronRDP/issues/1184)) ([d92dd382b3](https://github.com/Devolutions/IronRDP/commit/d92dd382b3fbaa163f355f6489db45ca8a3e7498)) 


