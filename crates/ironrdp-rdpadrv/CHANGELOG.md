# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.0.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpadrv-v0.0.0)] - 2026-10-30

### <!-- 1 -->Features

- Add MultiPoint audio level and drive letter persistence channels ([#2322](https://github.com/Devolutions/IronRDP/issues/2322)) ([8301ee04d3](https://github.com/Devolutions/IronRDP/commit/8301ee04d32f580f9d7582267a063de1812e1afb)) 

  Implements [MS-RDPADRV] (Audio Level and Drive Letter Persistence
  Virtual Channel Extension), which Windows MultiPoint Server uses to have
  the client remember the session's master volume levels and the drive
  letters assigned to redirected USB storage, and restore them on the next
  connection.
  
  ## New crate `ironrdp-rdpadrv`
  
  A core-tier (`no_std` + alloc, no I/O) crate for the two dynamic virtual
  channels:
  
  - `WMSAud` (`AudioLevelPdu`): `SAE_Started`, `SAE_VolumeChange` (data
  flow, `f32` volume, mute) and `SAE_RemoteConnect`.
  - `WMSDL` (`DriveLetterPdu`): `SADLE_Started` and
  `SADLE_SerializedCache`, a list of `NAME_DATA` / `VALUE_DATA`
  registry-style pairs. The decoder checks both markers and that
  `cbMessageData` matches `cbNameValueData`, and ignores the spec's
  trailing "Unused" bytes.
  - Client processors (`AudioLevelClient`, `DriveLetterClient`): they
  store what the server reports and answer `Started` / `RemoteConnect`
  with the stored values, render before capture (§3.1.5). With nothing
  stored they send no answer, and they never send unprompted.
  - Server processors (`AudioLevelServer`, `DriveLetterServer`): they open
  the protocol (`Started`, or `RemoteConnect` for a reconnect), give the
  values the client restores to a handler, and build change reports.
  
  ## Client wiring
  
  `ironrdp-client` gains a `multipoint` feature (included in `all`, and
  forwarded by `ironrdp` as `client-multipoint`):
  
  - `ChannelConfig::multipoint` / `Config::with_multipoint_settings` take
  a shared `MultipointSettings` handle.
  - `build_connector` seeds both processors from it on every connection,
  so auto-reconnects restore the latest values.
  - Each reported change updates the settings and emits
  `RdpOutputEvent::MultipointSettingsChanged` through
  `OutputEventSender::defer`, so the embedder can save them.
  
  `ironrdp-viewer` matches `RdpOutputEvent` exhaustively and builds with
  `client-all`, so it gets a log-only arm.
  
  ## Tests
  
  - `ironrdp-testsuite-core/tests/rdpadrv`: wire layouts with spec section
  citations, rejection cases (unknown data flow, mismatched cache sizes),
  the client answering rules, server loopback, and proptest round-trips.
  - `ironrdp-client` unit tests: a reported value is saved and restored by
  the next connection.
  - Fuzzing: decode and round-trip oracles for both PDUs.
  
  ## Notes for reviewers
  
  - The spec calls `NAME_DATA.cchName` both a byte length and a count of
  WCHARs. The codec treats it as bytes, matching the field definition. No
  Windows binary that implements WMSAud or WMSDL was available to check
  against.
  - §3.1.5 says a client must not redirect USB mass storage before the
  drive letter protocol initializes. That gating against `rdpeusb` is not
  enforced. It is documented on `ChannelConfig::multipoint` and left as a
  follow-up.
  - The crate starts at 0.0.0 without `publish = false`, because the
  published `ironrdp-client` depends on it (`ironrdp-rdpel` precedent).
  - Volume levels are only persisted. They are not applied to local
  `rdpsnd` playback.
  
  ## Not tested
  
  - No live MultiPoint server: the behavior is verified only against the
  spec through unit, loopback and fuzz tests.
  - Locally, `cargo xtask ci` passed fmt, typos, tests, lints, the feature
  matrix, dependencies, captures and fuzz.
  - `wasm check` stopped because `wasm2wat` is not installed on this
  machine.
  - `web check` failed Prettier on 52 web-client files this PR does not
  touch, which looks like Windows line endings.
  - One `testsuite-extra` e2e test
  (`an_auto_reconnect_cookie_the_embedder_issues_invalidates_the_previous_one`)
  hung once in the full run, then passed alone and in a full rerun of the
  suite.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------


