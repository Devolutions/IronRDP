# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.8.0...ironrdp-svc-v0.9.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Tolerate unrecognized device-reported USB capability values ([#1418](https://github.com/Devolutions/IronRDP/issues/1418)) ([b2d75d4862](https://github.com/Devolutions/IronRDP/commit/b2d75d4862b4549b0a63cf9b26f02c3e8813014b)) 

  ## Problem

- Support runtime-defined static virtual channels ([#1517](https://github.com/Devolutions/IronRDP/issues/1517)) ([8b4c483ba0](https://github.com/Devolutions/IronRDP/commit/8b4c483ba0c900a8de0b2718347754f56dd363ba)) 

  ## Summary
  - add keyed runtime-defined static-channel registration, lookup, and
  negotiated ID attachment
  - enforce the static-channel limit and reject malformed SVC fragment
  sequences
  - wire generic connector, acceptor, and session name-based dispatch
  support
  
  ## Testing
  - `cargo test -p ironrdp-testsuite-core --test integration_tests_core
  svc::`
  - `cargo clippy -p ironrdp-testsuite-core --test integration_tests_core
  -- -D warnings`
  
  ---------

- Negotiate static channel chunk sizing ([#1622](https://github.com/Devolutions/IronRDP/issues/1622)) ([4e3903fbbe](https://github.com/Devolutions/IronRDP/commit/4e3903fbbef2904505f35e3437a7106807ac5987)) 

  Use the validated server VCChunkSize for outgoing static virtual channel
  data and retain 1600-byte chunks when it is absent or invalid.
  
  Apply refreshed values after reactivation across native, web, and FFI
  active stages while preserving channel flags.

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

- [**breaking**] Redirect several printers with cached settings ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995)) 

  Printer redirection could announce only the Windows default queue. Every
  printer reused the `PRN1` port name and the default flag,
  `PAKID_PRN_CACHE_DATA` was dropped so server-side printing preferences
  were lost on reconnect, and the viewer ignored the `.rdp`
  `redirectprinters` property that the daemon honored.
  
  - **`ironrdp-rdpdr`:** `Rdpdr::with_redirected_printer` announces an
  `RdpdrPrinter` with its own `PRN<n>` port name, flags, and cached
  configuration. Only the first default printer keeps the default flag
  (MS-RDPEPC 2.2.2.1). `RdpdrBackendProduct` holds a list of printers and
  an optional `RdpdrPrinterCacheStore`. Cache-data PDUs are decoded in the
  new `pdu::epc` module, saved through the embedder's store, and replayed
  as `CachedPrinterConfigData` on the next announcement. A driver from an
  add event replaces the announced driver unless the user chose it.
  Entries are keyed by the local queue name and record the name the server
  gave the printer (a rename event, or a queue a user installed on the
  redirected port), which the next announcement uses (MS-RDPEPC 3.2.5.1.3,
  3.2.5.1.6); the local Windows queue is never renamed by the server. The
  channel does no I/O. Malformed cache and `PAKID_PRN_USING_XPS` PDUs fail
  the channel like other malformed RDPDR packets (MS-RDPEFS 3.1.5.2); a
  valid `PAKID_PRN_USING_XPS` is ignored because XPSFORMAT is never
  announced.
  - **`ironrdp-rdpdr-native`:** `WindowsPrinterRedirection` selects the
  default queue, every local queue and printer connection (`EnumPrintersW`
  with `PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS`, like FreeRDP), or
  named queues, with per-queue driver overrides. One worker serves all
  printers and binds each job to its device. Jobs keep the `RAW` datatype
  because the server sends device-ready PRN data (MS-RDPEPC 3.2.5.1.2); a
  queue default such as `NT EMF 1.008` or `XPS_PASS` would make the print
  processor reinterpret it. `FileRdpdrPrinterCacheStore` persists cache
  entries; it reads each entry once and writes on a background thread, so
  cache events do not block the RDPDR channel on disk I/O. The README
  records what Easy Print would need.
  - **`ironrdp-svc` / `ironrdp-session`:** chunked static-channel PDUs no
  longer always carry `CHANNEL_FLAG_SHOW_PROTOCOL`. The flag now comes
  only from the message or from a channel defined with
  `CHANNEL_OPTION_SHOW_PROTOCOL`, as in FreeRDP. A live Windows 11 host
  silently dropped every RDPDR device announcement larger than one
  1600-byte chunk, so printers announced with cached settings were never
  redirected. This departs from the MS-RDPBCGR 3.1.5.2.1 wording in favor
  of observed Windows and FreeRDP behavior.
  - **`ironrdp-client`:** `RdpdrConfig::printers` mirrors
  `redirectprinters` and can drop a product's printers; it defaults to
  true. The channel announces every printer and marks a lone printer as
  the default.
  - **`ironrdp-daemon`:** printer settings persist per server under
  `%APPDATA%\ironrdp-agent\printer-cache` (or
  `IRONRDP_AGENT_PRINTER_CACHE`).
  - **`ironrdp-viewer`:** adds `--printer-all`, `--printer-queue
  <queue>[,<driver>]`, and `--printer-default`. `redirectprinters:i:1` now
  redirects the default printer unless `ironrdp_rdpdr:i:0` is set, as in
  the daemon. mstsc writes that property by default, so stock `.rdp` files
  now redirect the default printer and save its server-side settings in
  the per-server cache folder; this is intended and should be called out
  in the release notes.
  
  There is no fallback driver: a Windows 11 host that lacks the announced
  driver still accepts the announcement and creates no queue, so the
  client cannot detect it. `--printer-queue <queue>,<driver>` picks a
  driver the host has.
  
  Easy Print (XPSFORMAT) is not implemented: it needs the MS-RDPEXPS
  `TSVCTKT` and `XPSRD` channels, PrintTicket conversion on the client,
  XPS mode, and local XPS printing.
  
  The public API changes incompatibly: `RdpdrPrinter::new` no longer marks
  the printer as the default, `RdpdrBackendProduct::with_printer` appends
  instead of replacing, `Devices::add_printer*` return whether a `PRN<n>`
  port name was left, and `RdpdrConfig` has a new public `printers` field.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.7.0...ironrdp-svc-v0.8.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.6.0...ironrdp-svc-v0.7.0)] - 2026-05-27

### <!-- 1 -->Features

- Add SvcMessage::encode_unframed_pdu for headerless encoding ([#1093](https://github.com/Devolutions/IronRDP/issues/1093)) ([a21378e16a](https://github.com/Devolutions/IronRDP/commit/a21378e16a3a5af36428ba9a226b08acc5113eb6)) 

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 


## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.4.0...ironrdp-svc-v0.4.1)] - 2025-06-27

### <!-- 1 -->Features

- Implement `fmt::Debug` of `SvcMessage` (#791) ([5482365655](https://github.com/Devolutions/IronRDP/commit/5482365655e5c171cd967eda401b01161a9f6602)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.3.0...ironrdp-svc-v0.4.0)] - 2025-05-27

### <!-- 7 -->Build

- Bump bitflags from 2.9.0 to 2.9.1 in the patch group across 1 directory (#792) ([87ed315bc2](https://github.com/Devolutions/IronRDP/commit/87ed315bc28fdd2dcfea89b052fa620a7e346e5a)) 


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.2.0...ironrdp-svc-v0.3.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.1.3...ironrdp-svc-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.1.2...ironrdp-svc-v0.1.3)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.1.1...ironrdp-svc-v0.1.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
