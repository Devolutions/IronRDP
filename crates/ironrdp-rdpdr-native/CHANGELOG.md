# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.7.0...ironrdp-rdpdr-native-v0.8.0)] - 2026-10-23

### <!-- 0 -->Security

- Add advanced Windows filesystem semantics ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61)) 

  Extend the Windows-native RDPDR backend with confined directory,
  notification, lock, security, stream, control, and volume support.
  Decode only the portable IRPs and capability needed to dispatch these
  native operations.

### <!-- 1 -->Features

- [**breaking**] Tolerate unrecognized device-reported USB capability values ([#1418](https://github.com/Devolutions/IronRDP/issues/1418)) ([b2d75d4862](https://github.com/Devolutions/IronRDP/commit/b2d75d4862b4549b0a63cf9b26f02c3e8813014b)) 

  ## Problem

- Add Windows filesystem backend ([#1587](https://github.com/Devolutions/IronRDP/issues/1587)) ([7dce8a306f](https://github.com/Devolutions/IronRDP/commit/7dce8a306f43462677879905642967066c42337f)) 

  Add a handle-relative Windows RDPDR backend for one selected volume.
  It confines protocol paths below an opened root and supports bounded
  file I/O
  and basic metadata.
  
  Unsupported advanced filesystem operations return STATUS_NOT_SUPPORTED;
  later
  stack layers will add advanced Windows semantics and host integration.

- Wire RDPDR backends into client connections ([#1600](https://github.com/Devolutions/IronRDP/issues/1600)) ([1fbc9bab0b](https://github.com/Devolutions/IronRDP/commit/1fbc9bab0bc26d8fe0789d5215005d7ea22e2a54)) 

  Build a fresh RDPDR backend product for every connection attempt.
  
  Attach RDPDR only when its product has filesystem devices, advertise
  RDPSND for Windows interoperability, and deliver deferred responses.

- Add static drive redirection ([#1616](https://github.com/Devolutions/IronRDP/issues/1616)) ([a724f783d1](https://github.com/Devolutions/IronRDP/commit/a724f783d1638b4e215507849c1cf148887f30d5)) 

  Expose Windows logical volumes through the ActiveX drive collection and
  configure a static RDPDR backend from the selected pre-connect snapshot.
  
  DisableRdpdr remains a hard override.

- Plumb smartcard device into RDPDR backends ([#1656](https://github.com/Devolutions/IronRDP/issues/1656)) ([66831bbbba](https://github.com/Devolutions/IronRDP/commit/66831bbbbabe3bf36bedff769c3e62819f60d46b)) 

  Return immediate SvcMessage completions from handle_scard_call so
  backends can finish MS-RDPESC IRPs without blocking the channel path.
  
  Wire WindowsRdpdrBackendFactory::with_smartcard and a minimal
  ScardSession stub that answers decoded calls with
  SCARD_E_UNSUPPORTED_FEATURE. Allow smartcard-only RDPDR products (no
  drives). Full WinSCard work and product CLI/ActiveX enablement remain
  follow-ups.
  
  Depends on #1654.

- Windows WinSCard core smartcard backend ([#1661](https://github.com/Devolutions/IronRDP/issues/1661)) ([1e68bf1a1f](https://github.com/Devolutions/IronRDP/commit/1e68bf1a1f74631c0a1e3dcd16ffb927cae2080c)) 

  Replace the PR2 Windows smartcard stub with a trimmed WinSCard core so
  logon-critical MS-RDPESC IOCTLs complete against the host resource
  manager.

- Extend Windows WinSCard IOCTL coverage ([#1668](https://github.com/Devolutions/IronRDP/issues/1668)) ([992f7d8f5e](https://github.com/Devolutions/IronRDP/commit/992f7d8f5ec46ed1faea7ab57d876811df9f8c42)) 

  Implement remaining MS-RDPESC WinSCard paths on the PR3 core backend:
  LocateCards/ByATR, Control, Get/SetAttrib, GetTransmitCount,
  Read/WriteCache, GetReaderIcon, GetDeviceTypeId, and ContextAndString*
  reader-group admin. Keep A→W execution, buffer probes, and deferred
  workers. Product wiring stays out of scope.
  
  Size gate (counted .rs only): +467/-12 = 479 lines, 2 files (size/L).
  PR5 still owns daemon/agent/viewer/ActiveX product wiring.

- Enable Windows smartcard in product surfaces ([#1672](https://github.com/Devolutions/IronRDP/issues/1672)) ([1561a4c327](https://github.com/Devolutions/IronRDP/commit/1561a4c327512a1a3a6c618d4b06dc388fab132f)) 

  Wire daemon, agent, viewer, and ActiveX to the WinSCard RDPDR backend so
  products can announce a smartcard device only with a matching factory.
  
  Expose `WindowsRdpdrBackendFactory::with_smartcard`, daemon/agent
  `--smartcard` and `ironrdp_smartcard` connect overrides, viewer
  `--smartcard`, and ActiveX `RedirectSmartCards`. Smartcard-only RDPDR is
  valid on Windows; non-Windows paths refuse or hard-disable enablement.

- Redirect dynamic drives ([#1780](https://github.com/Devolutions/IronRDP/issues/1780)) ([5e05637f06](https://github.com/Devolutions/IronRDP/commit/5e05637f0659d460d6e02b5567bb728f1d04585a)) 

  Keep drive capability active for hotplug-only sessions and route ActiveX
  drive rescans and selection changes through the live RDPDR channel.
  Preserve stable drive IDs, defer removals until server acknowledgement,
  and keep generic devices, printers, and ports explicitly unsupported.
  
  Smartcard redirection remains independent.

- Redirect default Windows printer ([#1876](https://github.com/Devolutions/IronRDP/issues/1876)) ([eaadd650c9](https://github.com/Devolutions/IronRDP/commit/eaadd650c9e2be98b57274b72095bf41da6ac3dd)) 

  Expose the preconnect RedirectPrinters setting and carry printer
  metadata through the RDPDR client factory.
  
  Discover the current Windows default queue, negotiate its driver
  metadata, and stream bounded RAW jobs on a module-pinned worker. Ports,
  generic PnP/USB devices, multiple printers, and Easy Print/XPS remain
  unsupported.

- Redirect a printer on Linux and macOS ([#2017](https://github.com/Devolutions/IronRDP/issues/2017)) ([d839ad3bd0](https://github.com/Devolutions/IronRDP/commit/d839ad3bd0e71dd44b68dd74bf122e108a5cbefb)) 

  Adds nix printer redirection: a factory announcing drives plus one virtual printer, and a PrinterSpooler answering printer create/write/close IRPs, spooling jobs privately (mkdtemp 0700, exclusive 0600 files) before handing them to lp or saving them.

- [**breaking**] Redirect serial and parallel ports (MS-RDPESP) ([#2187](https://github.com/Devolutions/IronRDP/issues/2187)) ([850839f475](https://github.com/Devolutions/IronRDP/commit/850839f4756f4efb486a47abb89a93b3b4694a40)) 

  Adds serial (COM) and parallel (LPT) port redirection ([MS-RDPESP]) over
  RDPDR, with a Windows backend, and wires it into the viewer, `.rdp`
  files, the agent daemon and ActiveX. The behavior follows mstsc
  (mstscax.dll 10.0.26100.9444) except where noted below.
  
  ## Protocol (`ironrdp-rdpdr`, core tier, no I/O)
  - `PORT_CAPS_SET` (`CapabilityMessage::new_port`,
  `Capabilities::add_port`, `CoreCapability::supports_port`).
  - Serial/parallel device announces (`Devices::add_port`): the DOS name
  goes in `PreferredDosName`, and there is no device data. They match the
  MS-RDPESP 4.1 annotations byte for byte.
  - `Rdpdr::with_port(RdpdrPort)`. Ports are only announced when the
  server advertises the port capability, at the same points as mstsc:
  - Serial ports are announced at Client ID Confirm, after the smart
  cards, in a Device List Announce of their own.
    - Parallel ports go in the post-logon batch with drives and printers.
  - When Windows sends Client ID Confirm before its capability request (it
  does this when it re-announces the channel for an existing session),
  serial ports are announced at User Logged On instead.
  - New `pdu::esp` module:
  - `PortIoCtlCode` with the MS-RDPESP IOCTL table, and `check_buffers`,
  which applies the same buffer checks mstsc makes before touching the
  device.
  - `PortIoRequest` for create, close, read, write, flush and device
  control.
  - Only the `ntddser.h` buffers those checks need (`SerialTimeouts`,
  `SerialQueueSize`, `SerialStatus`). IOCTL buffers are otherwise opaque,
  because the Windows backend forwards them unchanged; typed buffers can
  be added with the first client that interprets them.
  - Port IRPs go to the new `RdpdrBackend::handle_port_io_request`, which
  completes them with `STATUS_NOT_SUPPORTED` by default. The processor
  completes, rather than ignoring, major functions that MS-RDPESP does not
  describe (`STATUS_NOT_SUPPORTED`) and device controls whose
  InputBufferLength is longer than the PDU (`STATUS_INVALID_PARAMETER`).
  mstsc sends no completion at all in that second case.
  - `DeviceControlResponse::with_raw_output` carries output produced by a
  driver.
  - **Early IRPs:** port IRPs use #2191's `device_is_announced`, so a
  Create or DeviceControl that Windows sends before the announcement
  response is served instead of dropped.
  `handle_server_device_announce_response` is where a backend learns of a
  rejection, and the Windows backend releases the files and pending
  requests the server opened before rejecting the port, because the
  processor ignores the rejected port's closes.
  
  ## Windows backend (`ironrdp-rdpdr-native`)
  - **Selection:**
  `WindowsRdpdrBackendFactory::with_port_redirection(WindowsPortRedirection)`
  selects all, named or no ports of each kind. Ports are enumerated when
  each product is built. Like mstsc, only `COM<n>` serial and `LPT<n>`
  parallel ports can be named, so a name cannot hand another kind of
  device to the server.
  - **Opening:**
  - Every server create opens its own local handle (`NtCreateFile`,
  asynchronous) with the server's DesiredAccess, its ShareAccess without
  `FILE_SHARE_DELETE`, and its disposition. `FILE_SUPERSEDE` and
  `FILE_OVERWRITE` are rejected with `STATUS_INVALID_PARAMETER`.
  - A second open of an exclusive port therefore fails as it would
  locally.
  - A newly opened serial port gets its default configuration
  (`GetDefaultCommConfigW`) and mstsc's timeouts: ReadIntervalTimeout 200
  ms, ReadTotalTimeoutConstant 5 s, WriteTotalTimeoutConstant 60 s,
  multipliers unchanged. This is skipped when the default configuration
  cannot be read. Parallel ports are opened as-is.
  - **I/O:**
  - Reads, writes and IOCTLs are sent to the driver unchanged
  (`NtReadFile` / `NtWriteFile` / `NtDeviceIoControlFile`), after the
  `check_buffers` checks.
  - The driver's NTSTATUS and `Information` are returned as-is, as
  MS-RDPESP Appendix A describes. Errors carry no data; warnings such as
  `STATUS_BUFFER_OVERFLOW` keep partial output.
  - A `WAIT_ON_MASK` completion always carries its 4-byte mask, as in
  mstsc.
  - Reads and IOCTL output buffers are capped at 64 KiB, which partial
  reads allow.
  - **Pending requests:**
  - Requests the driver pends (reads, `IOCTL_SERIAL_WAIT_ON_MASK`)
  complete through `poll_deferred_messages`.
  - Each pending request has a worker thread with a 64 KiB stack that
  waits on the request's event. Workers acquire the host's
  `RdpdrWorkerThreadHooks` (ActiveX module lifetime). They hand the
  finished request back to the session, which keeps the request allocated
  until it retires the pending entry.
  - `IRP_MJ_CLOSE` cancels the file's pending requests
  (`NtCancelIoFileEx`) without blocking the channel thread. Their real
  completions, then the close completion, follow through
  `poll_deferred_messages`.
  - A request the driver hasn't completed 2 s after cancellation is
  reported as cancelled and left to its worker. A new initialization
  sequence abandons pending requests without waiting.
  - A flush completes once the file's earlier writes have finished.
  Deferred flushes count toward the pending-request limit.
  
  ## Wiring
  - `.rdp`: `redirectcomports` (`ironrdp-cfg` accessors) maps to the new
  `RdpdrConfig::ports` / `ConfigBuilder::with_ports`.
  - Client: `build_rdpdr_channel` announces the product's ports, honours
  the RD Gateway `HTTP_TUNNEL_REDIR_DISABLE_PORT` policy, and rejects
  device ID collisions.
  - Viewer: `--serial PORT` / `--parallel PORT` (repeatable, or `all`),
  like FreeRDP's `/serial:COM1` and `/parallel:LPT1`. Without them,
  `redirectcomports:i:1` redirects every port unless `ironrdp_rdpdr:i:0`.
  - Daemon/agent: a connect-time `redirectcomports:i:1` redirects every
  port.
  - ActiveX: `RedirectPorts` is implemented instead of returning
  `E_NOTIMPL`. It is persisted as `redirectcomports`, which is also
  accepted by `ApplySettings` / `SetRdpProperty`.

- Drive redirection for the viewer, .rdp files and Windows hotplug ([#2178](https://github.com/Devolutions/IronRDP/issues/2178)) ([1de38a2ee5](https://github.com/Devolutions/IronRDP/commit/1de38a2ee55346a577ec28f3706c81dd3f7e8316)) 

  Drive redirection used to be reachable only through ActiveX and the
  daemon's fixed volume list. This makes it usable from the viewer and
  from standard `.rdp` files, and adds automatic drive hotplug on Windows.
  
  ## Changes
  
  - **`ironrdp-cfg`**: `DriveRedirection` parses `drivestoredirect` with
  mstsc semantics (`*` = every drive including later ones,
  `DynamicDrives`, `C:\;E:\`). A nonempty `drivestoredirect` wins;
  otherwise `redirectdrives:i:1` selects every drive, as in FreeRDP. New
  `PropertySetExt` accessors for both keys plus `drive_redirection()`.
  - **`ironrdp-client`**: `RdpdrConfig::drives` and
  `ConfigBuilder::with_drive_redirection`, filled from the property set.
  The client only carries the selection; frontends build the native
  factory.
  - **`ironrdp-rdpdr-native`** (Windows):
  - A redirected root may now be a directory on a volume
  (`C:\Users\name`), not only `C:\`. Paths with `..`, `\?\`/device/UNC
  prefixes, `:` or NUL are rejected, and the root is still opened with
  `OBJ_DONT_REPARSE`.
  - `WindowsRdpdrBackendFactory::with_logical_drives` registers drive
  letters as device IDs 1–26 and announces the selected letters present
  when each connection (including reconnects) starts.
  - `LogicalDriveWatcher` watches `WM_DEVICECHANGE` on a hidden top-level
  window, plus a 2 s `GetLogicalDrives` poll for network drives, which are
  not broadcast. It reports arrivals and removals; arrivals are added to
  the shared selection.
  - **`ironrdp-viewer`**: `--drive NAME,PATH`, `--drives`, `--home-drive`,
  `--dynamic-drives`; honors the `.rdp` keys; attaches the RDPDR backend
  whenever any drive, smartcard or printer redirection is requested;
  forwards watcher changes as `AddRdpdrDrive` / `RemoveRdpdrDrive`.
  - **`ironrdp-daemon`**: honors `drivestoredirect` / `redirectdrives` on
  connect (an explicit `ironrdp_rdpdr:i:0` disables them) and keeps a
  watcher for the session's lifetime. Configured daemon volumes now use
  device IDs from 27, so they cannot collide with drive letters.
  
  ## Testing
  
  - `cargo xtask check fmt -v` and `cargo xtask check lints -v` pass.
  - New tests: `.rdp` parsing and precedence
  (`testsuite-core/tests/cfg.rs`), viewer CLI and `.rdp` mapping
  (`testsuite-extra/tests/client/config.rs`), root path validation,
  logical-drive factory, daemon factory.
  - Opt-in `watcher_reports_subst_drive_arrival_and_removal` (`--ignored`)
  maps a free letter with `subst` and passes locally: arrival and removal
  are both reported.
  - Not done: a live session against a server, and a physical USB stick.
  - `e2e::egfx_moves_onto_the_udp_tunnel_with_soft_sync` timed out once
  under a parallel run and passed 3/3 alone; it is unrelated to this
  change.
  
  ## Merge note
  
  This overlaps the printer redirection PR #2177 in `ironrdp-rdpdr-native`
  (`factory.rs`, exports, README), `ironrdp-client` config, and the
  viewer's `cli.rs` / `main.rs`. Whichever lands second needs a rebase.
  The device ID layout is: 0 smartcard, 1–26 drive letters, 27+ named
  drives, printers from `u32::MAX - 1` down.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)

- Redirect smartcards through WinSCard on Windows ([#2175](https://github.com/Devolutions/IronRDP/issues/2175)) ([b735d190dd](https://github.com/Devolutions/IronRDP/commit/b735d190ddfd40fe2ecdf818f96235f185f3420d)) 

  ## Problem
  
  `ClientConnector.WithStaticChannelRdpdr` in the .NET FFI announces a
  smartcard device backed by `NoopRdpdrBackend`. That backend's
  `handle_scard_call` returned no messages, so the client never completed
  any MS-RDPESC IOCTL the server sent (EstablishContext, ListReaders,
  GetStatusChange, ...). Server-side WinSCard calls in the session hung,
  which can stall logon and UAC prompts that check for readers.
  `ironrdp-client` attaches the same backend when RDPDR is enabled with no
  native backend.
  
  ## Fix
  
  **The noop backend answers every smartcard call.** It replies with the
  return structure the server expects for that IOCTL, with `ReturnCode =
  SCARD_E_NO_SERVICE` and `IoStatus = STATUS_SUCCESS`, as MS-RDPESC 3.1.4
  requires for smart card errors. Shared helpers in `ironrdp-rdpdr` build
  these replies, and the Windows backend now uses them too:
  
  - `ScardCall::error_response` builds the completion for a failed call.
  An IOCTL with no known return structure gets `STATUS_UNSUCCESSFUL` and
  no output, as in FreeRDP; this path is unreachable today.
  - `esc::complete_call` replies `STATUS_BUFFER_TOO_SMALL` when the
  structure does not fit the server's output buffer, so the server retries
  with a larger one (3.2.5.2).
  - Error returns zero every field besides `ReturnCode` (2.2.3): empty
  context and handle (`ScardContext::empty`, `ScardHandle::empty`), NULL
  buffers, and no reader states (kept on `SCARD_E_TIMEOUT`, as FreeRDP
  does). This also fixes failure replies in the Windows backend (Connect,
  Transmit, Reconnect, GetTransmitCount, GetDeviceTypeId).
  
  **Real WinSCard redirection in the FFI on Windows.**
  `WithStaticChannelRdpdr` now backs the smartcard with the WinSCard
  passthrough from `ironrdp-rdpdr-native`, the same backend the viewer and
  ActiveX use. Other platforms keep the noop backend.
  
  The native backend runs blocking calls (GetStatusChange, Connect,
  Transmit, ...) on worker threads, and their completions only reach the
  server through `Rdpdr::poll_deferred_messages`. So:
  
  - `ActiveStage.Process` sends deferred RDPDR completions after each
  frame, as `ironrdp-client` does, except after `DeactivateAll` or
  `Terminate`.
  - New `ActiveStage.PollRdpdrDeferred()` returns them while the server is
  idle. Hosts should call it about every 50 ms, serialized with their
  other `ActiveStage` calls.
  - The Avalonia example polls every 50 ms from its read loop, and starts
  the next read only after the current frame's outputs are handled, so
  reactivation reads are not raced.
  
  The existing .NET API is unchanged; `PollRdpdrDeferred` is an addition.
  The regenerated bindings only add the new method and doc comments.
  
  ## Related PRs
  
  This PR builds on #2161 and #2162, both now merged into master:

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



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.6.0...ironrdp-rdpdr-native-v0.7.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-rdpdr` public dependency to 0.7

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.5.0...ironrdp-rdpdr-native-v0.6.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Model CreateDisposition as enum instead of bitflags ([#1145](https://github.com/Devolutions/IronRDP/issues/1145)) ([c4f87aa417](https://github.com/Devolutions/IronRDP/commit/c4f87aa417e83c9cf6d1550c877ea3facb2f9a59)) 

  CreateDisposition values (FILE_SUPERSEDE through FILE_OVERWRITE_IF) are
  mutually exclusive integers 0 through 5, not combinable bit flags.
  Modeling them with the bitflags macro causes subtle correctness issues.

### <!-- 7 -->Build

- Bump nix from 0.30.1 to 0.31.1 ([#1085](https://github.com/Devolutions/IronRDP/issues/1085)) ([e92135dc0d](https://github.com/Devolutions/IronRDP/commit/e92135dc0d46bb3217ad26fcb82651c29e9c43c4)) 


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.4.0...ironrdp-rdpdr-native-v0.5.0)] - 2025-12-18


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.3.0...ironrdp-rdpdr-native-v0.4.0)] - 2025-08-29

### <!-- 7 -->Build

- Bump nix to 0.30 ([971ad922a5](https://github.com/Devolutions/IronRDP/commit/971ad922a51f78511243aaa885acdd8b1ed94b27)) 
- Bump ironrdp-pdu

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.1.2...ironrdp-rdpdr-native-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.1.1...ironrdp-rdpdr-native-v0.1.2)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 
