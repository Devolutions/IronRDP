# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.8.0...ironrdp-displaycontrol-v0.9.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Tolerate unrecognized device-reported USB capability values ([#1418](https://github.com/Devolutions/IronRDP/issues/1418)) ([b2d75d4862](https://github.com/Devolutions/IronRDP/commit/b2d75d4862b4549b0a63cf9b26f02c3e8813014b)) 

  ## Problem

- [**breaking**] Record byte offset on decode and encode error variants ([#1266](https://github.com/Devolutions/IronRDP/issues/1266)) ([a1f9189c30](https://github.com/Devolutions/IronRDP/commit/a1f9189c307516361a8faff6ecb7c1690b267998)) 

  ## Summary
  
  Records a byte offset on every `DecodeErrorKind` and `EncodeErrorKind`
  variant that can know one, so decode and encode errors surface the
  position in the input stream where the failure was detected. Reshaped
  twice after review; see "Review history" below if you reviewed an
  earlier shape.
  
  Contributes to the structured-fuzzing roadmap in #1120 by giving
  crash-replay analysis and Wireshark-style malformed-PDU reporting the
  byte-offset dimension that source `Location` ([#1262](https://github.com/Devolutions/IronRDP/issues/1262)) alone does not
  provide.
  
  ## API
  
  Variants that gain `offset: Option<usize>`:
  
  - `DecodeErrorKind::NotEnoughBytes { received, expected, offset }`
  - `DecodeErrorKind::InvalidField { field, reason, offset }`
  - `DecodeErrorKind::UnexpectedMessageType { got, offset }`
  - `DecodeErrorKind::UnsupportedVersion { got, offset }`
  - `DecodeErrorKind::UnsupportedValue { name, value, offset }`
  - `EncodeErrorKind` mirrors the same shape for the encode side

- [**breaking**] Populate decode/encode error offsets from cursor positions ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec)) 

  ## Summary
  
  The workspace sweep that follows #1266. Decode and encode error
  construction sites now pass the cursor, so the reported position is the
  byte the decoder or encoder actually stopped at.
  
  Stacked on #1266 and merges after it.
  
  ## What "no position" means here
  
  #1266 makes `offset` an `Option<usize>` where `None` means the error has
  no position in the input stream at all, rather than a position that
  happened to be unavailable. This PR is the other half of that: it walks
  the workspace and gives a real position to every site that has one, so
  the sites left reporting `None` are the ones that genuinely never had
  one.
  
  Those are constructors validating their arguments, integer conversions,
  cache lookups that missed, accessors on already-decoded structures, and
  the declared-size checks described below. They report nothing rather
  than byte zero, and that is now their permanent answer rather than a gap
  awaiting another sweep.
  
  There are no `at: 0` sites left anywhere in the workspace.
  
  ## The rule
  
  The position is attached where the cursor identifies the bytes being
  complained about. It is omitted where the complaint is about a size the
  peer declared, computed from data already consumed, because there the
  cursor points at a byte that is not the problem.

- Let a server handler override its capabilities ([#1917](https://github.com/Devolutions/IronRDP/issues/1917)) ([db0066072e](https://github.com/Devolutions/IronRDP/commit/db0066072e787ea1268acf2ce46ed5ff0bb578b9)) 

  ## Summary
  
  - `DisplayControlServer::start()` sends
  `DisplayControlCapabilities::new(1, 3840, 2400)` unconditionally, so
  every server using this crate is capped at one monitor with no way to
  advertise more.
  - Added a default trait method on `DisplayControlHandler`,
  `capabilities()`, that `start()` now calls instead of the hardcoded
  literal.
  - The default returns the same `(1, 3840, 2400)` values, so any existing
  implementation that does not override it keeps its current behavior
  unchanged.
  - A handler serving more than one monitor can override `capabilities()`
  to report the real count.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  - Additive only: one new trait method with a default body, one call-site
  change in `start()`. No existing implementor is affected unless it opts
  in.

- Let RdpServerDisplay report its monitor count ([#1918](https://github.com/Devolutions/IronRDP/issues/1918)) ([25331598f3](https://github.com/Devolutions/IronRDP/commit/25331598f37f49573205c54ee45c878d291317f2)) 

  ## Summary
  
  - DisplayControlBackend (ironrdp-server's internal DisplayControlHandler
  implementor) never overrode capabilities(), so it fell through to the
  single-monitor default regardless of how many monitors the consumer's
  RdpServerDisplay actually serves.
  - Added a default RdpServerDisplay::monitor_count() method (returns 1,
  matching today's behavior), fetched once alongside size() before the
  dynamic channels are attached.
  - Threaded the count into DisplayControlBackend so its capabilities()
  now reports the real value instead of the hardcoded literal.
  - A consumer serving more than one monitor can override monitor_count()
  to report the real total.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  - Builds on #1917 (DisplayControlHandler::capabilities()), now merged;
  rebased onto master.
  - Additive only: one new trait method with a default body, one new
  field, one call-site change. No existing implementor is affected unless
  it opts in.

- [**breaking**] Multi-monitor layouts with orientation and DPI ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857)) 

  Display Control (MS-RDPEDISP) was only usable for a single primary
  monitor at 100% device scale. This lets clients send complete monitor
  layouts during the session, with real orientation and DPI, and wires it
  into the viewer and the ActiveX control.
  
  ## Changes
  
  **`ironrdp-displaycontrol`** (breaking)
  - `DisplayControlMonitorLayout::new` now rejects overlapping monitors
  and, for multi-monitor layouts, monitors not adjacent to another one
  (MS-RDPEDISP 3.1.5.2). Layouts it previously accepted can now fail.
  - `fit_to_capabilities` enforces the server's `MaxNumMonitors` and
  maximum monitor area (MS-RDPEDISP 3.2.5.2): extra monitors are dropped
  (the primary is kept), a single oversized monitor is scaled down keeping
  its aspect ratio, and an oversized multi-monitor layout is rejected
  rather than shrunk (which would move monitors relative to each other).
  - `DisplayControlClient` stores the server capabilities and gains
  `encode_monitor_layout`; `encode_single_primary_monitor` fits to the
  capabilities too.
  - `new_single_primary_monitor` clamps the desktop scale to 100..=500 and
  derives the device scale (100/140/180) from it instead of hard-coding
  100%; a square monitor is landscape.
  - Helpers: capability getters, `area`, `desktop_size`,
  `MonitorLayoutEntry::adjust_desktop_scale_factor`,
  `DeviceScaleFactor::from_value` / `from_desktop_scale_factor`.



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.7.0...ironrdp-displaycontrol-v0.8.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.8

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.6.0...ironrdp-displaycontrol-v0.7.0)] - 2026-06-05

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.5.0...ironrdp-displaycontrol-v0.6.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-core`, `ironrdp-dvc`, `ironrdp-pdu`, and `ironrdp-svc` public dependencies

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.3...ironrdp-displaycontrol-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.2...ironrdp-displaycontrol-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.1...ironrdp-displaycontrol-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.0...ironrdp-displaycontrol-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
