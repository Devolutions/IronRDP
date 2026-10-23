# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cfg-v0.1.0...ironrdp-cfg-v0.2.0)] - 2026-10-23

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

- [**breaking**] Tolerate unrecognized device-reported USB capability values ([#1418](https://github.com/Devolutions/IronRDP/issues/1418)) ([b2d75d4862](https://github.com/Devolutions/IronRDP/commit/b2d75d4862b4549b0a63cf9b26f02c3e8813014b)) 

  ## Problem

- Add RemoteApp channel support ([#1637](https://github.com/Devolutions/IronRDP/issues/1637)) ([ab48c6cb8c](https://github.com/Devolutions/IronRDP/commit/ab48c6cb8c017504f8a92799aeb91b821c50a13a)) 

  Configure and negotiate RAIL connections, then route its static channel
  through the portable client with bounded request queues and server
  control events.

- Wire MS-RDPEAI capture into Windows client and ActiveX ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d)) 

  Add the client MS-RDPEAI capture path on top of hardened RDPSND
  playback: connector CFG + static channel wiring, CPAL PCM capture
  backend, ironrdp-client --audio-capture, and ActiveX
  AudioCaptureRedirectionMode.
  
  PCM capture only accepts encode formats that match the Open capture
  stream, rejects non-16-bit capture (Data PDU size contract), and gates
  the capture backend behind ironrdp-rdpsnd-native/capture.
  
  Depends on #1648 (playback).

- Add native MS-RDPEWA WebAuthn redirection ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb)) 

  Implement the RDPEWA dynamic channel with a Windows WebAuthn backend and
  wire RedirectWebAuthn for ActiveX, the optional client feature, and the
  viewer CLI.
  
  Prefer System32\webauthn.dll via the DVC COM plugin for MSTSC parity.
  The pure-Rust backend forwards ceremonies through a webauthn.dll IWTS
  oneshot so hash-only hosts that omit clientDataJSON still work; public
  WebAuthN* remains a fallback when JSON is present. Recreate
  WebAuthN_Channel opens through shared COM/listener factories because
  Windows opens and closes the channel around each RPC.
  
  Side effects:
  - New crates ironrdp-rdpewa and ironrdp-rdpewa-native
  - Config key redirectwebauthn; ActiveX ExtendedSettings property
  - ironrdp-daemon webauthn feature for ironrdp-agent
  - IRONRDP_WEBAUTHN_FORCE_NATIVE debug switch
  - Viewer --webauthn/--no-webauthn flags; .rdp redirectwebauthn default
  - ActiveX docs note no AdvancedSettings slot and no IPersist persistence

- Try a direct connection before the RD Gateway ([#1713](https://github.com/Devolutions/IronRDP/issues/1713)) ([a67dbf47e4](https://github.com/Devolutions/IronRDP/commit/a67dbf47e429a73b29e2b36966b6dc5b870625cd)) 

- Wire local VMConnect features ([#1768](https://github.com/Devolutions/IronRDP/issues/1768)) ([cf9bdef764](https://github.com/Devolutions/IronRDP/commit/cf9bdef764da6b9aff33b436662441e1503dc93a)) 

  Wire VMConnect modes through the client configuration and RDP connector.
  Support current-user host authentication without a password, preserve
  guest credential boundaries, and attach Hyper-V framebuffer redirection
  only for eligible local Windows sessions.

- Redirect the default Windows printer ([#2156](https://github.com/Devolutions/IronRDP/issues/2156)) ([408b86ea91](https://github.com/Devolutions/IronRDP/commit/408b86ea91dc6b953778278ee2ffbd99756688f5)) 

  Windows default-printer redirection ([#1876](https://github.com/Devolutions/IronRDP/issues/1876)) was only reachable through
  the ActiveX control's `RedirectPrinters` setting. This exposes it in
  `ironrdp-viewer` and `ironrdp-daemon`.
  
  ## Changes
  
  - **`ironrdp-cfg`:** add `redirect_printers` / `set_redirect_printers` /
  `clear_redirect_printers` for the standard `redirectprinters` `.rdp`
  property.
  - **`ironrdp-viewer`:** add `--printer`. The Windows backend factory now
  asks for the default printer (`with_default_printer`). A `.rdp` file's
  `redirectprinters` property does not enable it, because mstsc writes
  `redirectprinters:i:1` by default and would otherwise announce the local
  printer for any stock file. The README documents the flag and the
  matching-driver requirement.
  - **`ironrdp-daemon`:** read `redirectprinters` on each connect and
  build a printer-only factory on demand. An explicit `ironrdp_rdpdr:i:0`
  overrides it. Non-Windows hosts log a warning and ignore the property.
  - **`ironrdp-agent`:** add `redirectprinters` to the property help
  table.
  - **`ironrdp-client` (gateway fix):** honor
  `HTTP_TUNNEL_REDIR_DISABLE_PRINTER` (MS-TSGU 2.2.5.3.7). The client
  checked only the drive and clipboard flags, so a printer was announced
  even when the RD Gateway disabled printer redirection.
  `build_rdpdr_channel` now takes an `RdpdrDevicePolicy` for drives and
  printers instead of a single drive bool. This also covers the existing
  ActiveX path.
  
  It uses the existing backend unchanged: the current default queue only,
  RAW jobs, and the remote host needs a printer driver with the same name
  as the local queue's driver. Multiple printers and Easy Print/XPS are
  not part of this PR.
  
  ## Testing
  
  - `cargo test -p ironrdp-client --lib`: new tests cover a printer-only
  product under printer-restricted policy and the `disables_printer`
  gateway flag.
  - `cargo test -p ironrdp-daemon rdpdr`: new test for the printer-only
  factory.
  - `cargo test -p ironrdp-testsuite-extra --test integration_tests_extra
  client::config`: new test that the viewer redirects only with
  `--printer`, including when a stock file sets `redirectprinters:i:1`.
  - `cargo xtask check fmt` and `cargo xtask check lints` pass locally on
  Windows. `check typos` was not run locally.
  - Not tested end to end against a server with a matching driver.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

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



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-cfg-v0.1.0)] - 2026-07-10

Initial release.
