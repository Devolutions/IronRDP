# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.7.0...ironrdp-cliprdr-native-v0.7.1)] - 2026-10-23

### <!-- 1 -->Features

- Add a Wayland data-control clipboard client ([#2055](https://github.com/Devolutions/IronRDP/issues/2055)) ([a1efb0bb85](https://github.com/Devolutions/IronRDP/commit/a1efb0bb85e1654ba91f1407e77050082ee3d7d2)) 

  This adds the data-control clipboard client I committed to on #2016. It
  lives in ironrdp-cliprdr-native as the data_control module, so a Linux
  CLIPRDR backend can be built on it.
  
  The client speaks ext-data-control-v1 and wlr-data-control-unstable-v1
  and uses whichever the compositor offers. One thread owns the Wayland
  connection and DataControl is the handle to call from anywhere. It has
  no async runtime.
  
  It supports delayed rendering. A type advertised without data raises a
  TransferRequest when something pastes it, which maps onto the CLIPRDR
  Format Data Response. Dropping the request without answering it closes
  the paste with no data. A selection this client set itself isn't
  reported back as a local copy, so a backend doesn't echo its own
  clipboard to the server. The client tells its own selection from another
  client's copy by one private MIME type that it advertises next to the
  real ones, so other clients see that type in the list they are offered.
  
  Reads are capped at 100 MiB and fail with a timeout if the source stalls
  for 5 seconds. Pastes are bounded too. At most 16 are served at once, a
  paste whose data isn't supplied within 5 seconds is closed with no data
  (an answer that comes later is kept for the next paste), and a reader
  that takes nothing for 5 seconds is dropped. MIME matching tolerates
  charset parameters. GNOME's Mutter offers no data-control protocol, so
  connecting returns an Unsupported error there.

- Add a Linux clipboard backend ([#2016](https://github.com/Devolutions/IronRDP/issues/2016)) ([aed5ce8ba2](https://github.com/Devolutions/IronRDP/commit/aed5ce8ba225c6d2b17ab39b0d12b3fc65e565bd)) 

  Adds native Linux clipboard redirection for Unicode text and images,
  built on the `data_control` Wayland client from #2055 (now on master)
  with an X11/XFixes fallback.
  
  Remote format lists are advertised immediately; contents are requested
  only when a local application pastes. Local selection changes are
  event-driven, and local content is read only when the RDP peer requests
  it. The backend uses the existing clipboard loop detector and the
  bounded bitmap converters in `ironrdp-cliprdr-format`.
  
  CLIPRDR responses have no request identifier. Only one Format Data
  Request is outstanding at a time: a timed-out OS paste is released,
  while the wire request remains reserved until its late response is
  drained. Selection generations reject obsolete data. If the peer never
  answers, remote pastes fail until channel reinitialization instead of
  applying stale content to a later copy.
  
  The X11 side bounds pending transfers, supports INCR for large content
  and isolates reads with separate requestor windows. Ownership
  acknowledgments and current-owner checks prevent queued events from
  replacing a newer remote selection.
  
  The only change to the `data_control` module is a crate-internal
  `synchronize()` (a `wl_display.sync` round trip). The backend calls it
  after `set_selection` so that `owns_selection()` reflects whether the
  selection was actually taken before the remote formats are treated as
  mirrored. If it was not, the backend announces the local formats again.
  
  File transfer and HTML are not implemented. Desktops without
  data-control use X11/XWayland when available, otherwise the existing
  stub backend.

- Transfer files through the Windows clipboard ([#2179](https://github.com/Devolutions/IronRDP/issues/2179)) ([b9d1400033](https://github.com/Devolutions/IronRDP/commit/b9d140003323f3ae3cd22f91be556e58ed43322e)) 

  Adds clipboard file transfer, in both directions, to the Windows
  clipboard backend (`WinClipboard`), and stops it advertising formats it
  can't deliver. ActiveX and `ironrdp-client` get the feature as is; the
  .NET FFI gets it through new bindings, opt-in.
  
  ## Backend (`feat(cliprdr)`)
  
  **Remote to local.** When the remote format list has
  `FileGroupDescriptorW`, an OLE data object goes on the clipboard
  (`OleSetClipboard`). It offers `FileGroupDescriptorW` and one
  `FileContents` `IStream` per file (`lindex`), plus the other remote
  formats.
  - Streams read through `FileContents` RANGE requests carrying the
  `clipDataId` of the lock `Cliprdr` took for that list, so a paste keeps
  working after the remote clipboard changes, until the lock is released.
  Files over 4 GiB work.
  - OLE needs an STA: this is only enabled when `WinClipboard` is created
  on one. COM is never initialized on a thread that has none.
  - The objects are not agile and run on the clipboard thread. A stream
  read blocks it like `WM_RENDERFORMAT` handling already does (30 s
  timeout, chunks up to 1 MiB with read-ahead). Backend events arriving
  during a wait are now queued instead of dropped.
  
  **Local to remote.** `CF_HDROP` paths (directories walked into relative
  paths), or another application's virtual files (`FileGroupDescriptorW` +
  `FileContents`), are offered with `SendInitiateFileCopy` once the
  channel is ready. The remote's SIZE/RANGE requests are served, from the
  lock snapshot when a `clipDataId` is given.
  
  **Capabilities.** The backend now negotiates `STREAM_FILECLIP_ENABLED`,
  `FILECLIP_NO_FILE_PATHS`, `CAN_LOCK_CLIPDATA` and
  `HUGE_FILE_SUPPORT_ENABLED`.
  
  **Format filtering.** Handle-based formats (`CF_BITMAP`,
  `CF_ENHMETAFILE`, `CF_METAFILEPICT`, `CF_PALETTE`), private ranges,
  local paths and shell items (`CF_HDROP`, `FileNameW`, `Shell IDList
  Array`) and OLE storage formats are no longer advertised in either
  direction. Images still go through as `CF_DIB`, which Windows
  synthesizes.
  
  **Request validation.** `ironrdp-client` treats a rejected clipboard
  request as session-fatal, so the backend checks what it sends against
  the rules `Cliprdr` enforces: file list indexes, range bounds, huge-file
  positions, and released locks.
  
  **Also fixed.** The window subclass state leaked on drop, and
  `WM_DESTROY` freed it while `WM_NCDESTROY` could still use it.
  
  ## Viewer (`fix(viewer)`)
  
  Without a backend factory, `ironrdp-client` created `WinClipboard` on
  the RDP thread, which runs a tokio runtime and no Windows message loop,
  so clipboard redirection never worked in the viewer (no clipboard
  events, no backend wake-ups, no STA for OLE). The viewer now initializes
  OLE and creates the clipboard, with file transfer, on its winit thread
  and hands the factory to `RdpClient`.
  
  ## .NET FFI (`feat(ffi)`)
  
  - `ActiveStage.SubmitClipboardMessage` hands any backend message to the
  clipboard channel, file transfer included, and returns the frame to
  send.
  - `ActiveStage.DriveClipboardTimeouts` expires clipboard locks and file
  transfers.
  - `WinCliprdr.NewWithFileTransfer(bool)` opts in. `WinCliprdr.New()`
  keeps file transfer off, so existing consumers that only route the three
  original message types don't receive new ones.
  - `WinCliprdr.NextClipboardMessageTimeout(ms)` lets a loop wake up to
  drive the timeouts.
  - The Avalonia example opts in, submits every message through
  `SubmitClipboardMessage`, drives the timeouts, and logs a failed
  clipboard operation instead of stopping clipboard handling.
  
  Only the four affected generated files are included.
  
  ## Testing
  
  - `cargo xtask check fmt`, `check lints` and `check locks` pass.
  - 12 unit tests in `cliprdr_native::windows`: format filtering, file
  list building, file contents serving, range sizing, descriptor
  round-trips.
  - 6 tests in `cliprdr_native::windows_live` run a `CliprdrClient` over
  `WinClipboard` against an in-memory `CliprdrServer` on the real Windows
  clipboard. They are opt-in with `IRONRDP_WIN_CLIPBOARD_LIVE=1`, because
  they replace the clipboard and briefly open an Explorer window. All pass
  locally:
  - Explorer pastes remote folders and files, contents checked byte for
  byte.
    - A 5 GiB remote file is read through OLE past 4 GiB.
    - Remote handle and path formats are not offered locally.
  - Local folders reach the remote, both at connect time and when copied
  mid-session.
    - Local image formats are filtered.
    - Virtual files from another session reach the remote.
  - .NET: the solution builds and the smoke tests pass (9/9, one new).
  
  - End to end with `ironrdp-viewer` against a Windows 11 server
  (rdpclip): capabilities negotiated as `STREAM_FILECLIP_ENABLED |
  FILECLIP_NO_FILE_PATHS | CAN_LOCK_CLIPDATA | HUGE_FILE_SUPPORT_ENABLED`.
  A local folder (nested directories, an empty directory, an empty file, a
  5 MiB random file) copied in Explorer was pasted into Explorer in the
  session, served under rdpclip's lock. Copying it back in the session and
  pasting it into a local Explorer window gave byte-identical files
  (SHA-256).
  
  Not verified by hand: ActiveX and the Avalonia example against a real
  server.
  
  ## Notes for review
  
  - This conflicts with #2175 in `MainWindow.axaml.cs` and the generated
  `ActiveStage` files. The clipboard loop body here is a single
  `SubmitClipboardMessage` call, so the merge only needs #2175's
  `WithActiveStage(...)` wrapper around it; the generated files can be
  regenerated.
  - Running the extra test suite locally may need `CMAKE` pointed at
  Visual Studio 2026's bundled CMake: `libopus_sys` fails with older CMake
  versions that lack the "Visual Studio 18 2026" generator.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

### <!-- 4 -->Bug Fixes

- Key auto-detect optional fields off requestType, not the Option ([#1491](https://github.com/Devolutions/IronRDP/issues/1491)) ([f9cc62fa2c](https://github.com/Devolutions/IronRDP/commit/f9cc62fa2ccb4f211838dd0961c19cdf3b79a38e)) 

  ## Summary
  
  MS-RDPBCGR decides which optional fields an auto-detect message carries
  by its `requestType`. Two of these message types encoded them by
  inspecting which `Option`s happened to be set instead, so the encoder
  and the decoder disagreed about the wire.
  
  This started as a fix for `BandwidthMeasureStop` alone. Review found the
  connect-time fallback was still non-compliant, and checking whether the
  same shape appeared elsewhere in the file turned up
  `NetworkCharacteristicsResult` with the identical defect and a worse
  consequence, so both are fixed here rather than one now and one later.
  
  ## The two failure modes
  
  **Connect-time stop with no payload: encodes to bytes the decoder
  rejects.**
  
  ```rust
  AutoDetectRequest::BandwidthMeasureStop {
      sequence_number: 7,
      request_type: BW_STOP_CONNECT_TIME,
      payload: None,
  }
  ```
  
  encodes to ten bytes with `headerLength` 0x06 and no length field.
  Decoding those bytes
  fails with `not enough bytes provided to decode: received 0 bytes,
  expected 2 bytes`,
  because the decoder reads `payloadLength` back for every
  `BW_STOP_CONNECT_TIME`.
  
  **UDP stop with a payload: silently loses it.**
  
  ```rust
  AutoDetectRequest::BandwidthMeasureStop {
      sequence_number: 3,
      request_type: BW_STOP_RELIABLE_UDP,
      payload: Some(vec![0xAA; 16]),
  }
  ```
  
  encodes the length and the sixteen bytes, which the decoder never reads
  back for
  `BW_STOP_RELIABLE_UDP` or `BW_STOP_LOSSY_UDP`.
  `AutoDetectReqPdu::decode` does not check
  for trailing bytes, so the payload is dropped in transit without an
  error rather than
  refused.
  
  The second is the worse of the two: a round trip that loses data and
  reports success.
  
  ## Network Characteristics Result (2.2.14.1.5)
  
  The same defect, found by sweeping the file rather than reported.
  
  `0x0840` carries baseRTT and averageRTT, `0x0880` carries bandwidth and
  averageRTT, `0x08C0` carries all three, and the decoder already read
  them on that basis. Encoding from the `Option`s meant:
  
  - a `0x0840` result with no `base_rtt_ms` wrote a body the decoder
  cannot read;
  - a `0x0840` result carrying `bandwidth_kbps` instead wrote that
  bandwidth into the slot the decoder reads as baseRTT, so the value came
  back **silently corrupted** rather than rejected;
  - `headerLength` was always derived from `requestType`, so it could
  contradict the body it described.
  
  Encode and `size()` now consult `requestType` through a shared helper, a
  value the type does not carry is dropped rather than written, and a
  missing one that the type requires is an error.
  
  ## Fix
  
  `Encode` and `size()` now key off `requestType`, matching the decoder.
  That makes the
  wire form canonical:
  
  - an absent payload on a connect-time stop encodes as a zero length and
  reads back as an
    empty one;
  - a payload on a UDP stop never reaches the wire.
  
  Decoding and re-encoding reproduces the same bytes in every case. The
  types can still
  express states the protocol cannot, so value-level identity is not
  achievable, but byte
  stability is, and that is what a decoder consuming real traffic depends
  on.
  
  No public signatures change; this is a behaviour fix inside the existing
  `Encode` impl.
  
  ## Tests
  
  New `pdu/autodetect.rs` in `ironrdp-testsuite-core`:
  
  - a connect-time stop with no payload round-trips, and `size()` agrees
  with `encode()`;
  - a UDP stop carrying a payload encodes header-only and comes back with
  `payload: None`,
    for both the reliable and lossy request types;
  - a connect-time stop preserves a real payload;
  - decode-then-encode reproduces identical bytes for both shapes.
  
  Three of the four fail against the current encoder and pass with the
  fix. The fourth is
  a control that passed before and after.
  
  Note the existing `pdu_round_trip` fuzz oracle would not have caught
  this even with
  auto-detect added to it, since it discards the result of the re-decode
  (`let _ =`). That
  is deliberate in the oracle's design and out of scope here, but it is
  why this went
  unnoticed.
  
  ## How this was found
  
  Writing a test for the connect-time bandwidth measurement in #1465,
  which needs to answer
  a connect-time stop. The `None` case was constructed to check the reply
  path stayed
  lenient and turned out not to be constructible on the wire at all.
  
  ## Verification
  
  - `cargo xtask check fmt -v` green
  - `cargo xtask check lints -v` green
  - `cargo xtask check tests -v` green
  - `cargo xtask check typos -v` green
  - `cargo xtask check locks -v` green

- Don't log OK error from GlobalUnlock ([#2111](https://github.com/Devolutions/IronRDP/issues/2111)) ([a50ad11dc9](https://github.com/Devolutions/IronRDP/commit/a50ad11dc9e09ebf1d6040cb5193605e8174f4b5)) 



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.6.0...ironrdp-cliprdr-native-v0.7.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-cliprdr` public dependency to 0.7



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.5.0...ironrdp-cliprdr-native-v0.6.0)] - 2026-05-27

### <!-- 1 -->Features

- Implement clipboard file transfer support ([#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9)) 

  Add end-to-end clipboard file transfer (upload and download) across the
  CLIPRDR channel per MS-RDPECLIP.

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.4.0...ironrdp-cliprdr-native-v0.5.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- Prevent window class registration error on multiple sessions ([#1047](https://github.com/Devolutions/IronRDP/issues/1047)) ([a2af587e60](https://github.com/Devolutions/IronRDP/commit/a2af587e60e869f0235703e21772d1fc6a7dadcd)) 

  When starting a second clipboard session, `RegisterClassA` would fail
  with `ERROR_CLASS_ALREADY_EXISTS` because window classes are global to
  the process. Now checks if the class is already registered before
  attempting registration, allowing multiple WinClipboard instances to
  coexist.

### <!-- 7 -->Build

- Bump windows from 0.61.3 to 0.62.1 ([#1010](https://github.com/Devolutions/IronRDP/issues/1010)) ([79e71c4f90](https://github.com/Devolutions/IronRDP/commit/79e71c4f90ea68b14fe45241c1cf3953027b22a2)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.3.0...ironrdp-cliprdr-native-v0.4.0)] - 2025-08-29

### <!-- 4 -->Bug Fixes

- Map `E_ACCESSDENIED` WinAPI error code to `ClipboardAccessDenied` error (#936) ([b0c145d0d9](https://github.com/Devolutions/IronRDP/commit/b0c145d0d9cf2f347e537c08ce9d6c35223823d5)) 

  When the system clipboard updates, we receive an `Updated` event. Then
  we try to open it, but we can get `AccessDenied` error because the
  clipboard may still be locked for another window (like _Notepad_). To
  handle this, we have special logic that attempts to open the clipboard
  in the event of such errors.
  The problem is that so far, the `ClipboardAccessDenied` error was not mapped.

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.3...ironrdp-cliprdr-native-v0.1.4)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.2...ironrdp-cliprdr-native-v0.1.3)] - 2025-02-03

### <!-- 4 -->Bug Fixes

- Handle `WM_ACTIVATEAPP` in `clipboard_subproc` ([#657](https://github.com/Devolutions/IronRDP/issues/657)) ([9b2926ea12](https://github.com/Devolutions/IronRDP/commit/9b2926ea1212d3f9dec9354334d5bdaa1bebd81e)) 

  Previously, the function handled only `WM_ACTIVATE`.

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.1...ironrdp-cliprdr-native-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo ([#631](https://github.com/Devolutions/IronRDP/issues/631)) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.0...ironrdp-cliprdr-native-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
