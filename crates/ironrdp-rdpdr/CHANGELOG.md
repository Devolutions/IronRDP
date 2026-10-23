# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.7.0...ironrdp-rdpdr-v0.8.0)] - 2026-10-23

### <!-- 0 -->Security

- Add advanced Windows filesystem semantics ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61)) 

  Extend the Windows-native RDPDR backend with confined directory,
  notification, lock, security, stream, control, and volume support.
  Decode only the portable IRPs and capability needed to dispatch these
  native operations.

### <!-- 1 -->Features

- Add filesystem PDU foundation ([#1566](https://github.com/Devolutions/IronRDP/issues/1566)) ([161409e18d](https://github.com/Devolutions/IronRDP/commit/161409e18dd185de9bb30730303e56f5e8d28941)) 

  ## Summary
  - Add portable MS-RDPEFS/MS-FSCC filesystem request and completion
  codecs with malformed-input validation and wire tests.
  - Keep RDPDR runtime dispatch, Windows-native backend implementation,
  and client/session integration out of this foundation.
  
  ## Follow-up
  Later stacked PRs provide the backend implementation and runtime
  integration.
  
  ---------

- Add filesystem backend dispatch ([#1578](https://github.com/Devolutions/IronRDP/issues/1578)) ([8f5f3e2515](https://github.com/Devolutions/IronRDP/commit/8f5f3e25154a5edd324a8e89e30ef509a682dbaa)) 

  Route confirmed filesystem requests through portable backend contracts
  and make lifecycle, completion, and announcement state explicit.
  Validate filesystem close padding, release dynamically activated drives
  after rejected announcements, and prevent raw device removal from
  bypassing backend cleanup. The noop backend now rejects unsupported
  filesystem I/O.
  
  Later stacked PRs supply the concrete Windows backend and host
  integration.

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

- Complete MS-RDPESC PDUs and native handles ([#1654](https://github.com/Devolutions/IronRDP/issues/1654)) ([9f3ab27f88](https://github.com/Devolutions/IronRDP/commit/9f3ab27f887a2ee291afaffc97ace455b02e1ac7)) 

  Fill out the remaining MS-RDPESC call/return PDUs and encode
  SCARDCONTEXT/SCARDHANDLE as variable-length native values so x86
  (4-byte) and x64 (8-byte) WinSCard handles round-trip on the wire. Keep
  this change PDU-only; the WinSCard backend lands in stacked follow-ups.

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

- Make the PDU layer bidirectional for a future server ([#1779](https://github.com/Devolutions/IronRDP/issues/1779)) ([a993f91408](https://github.com/Devolutions/IronRDP/commit/a993f914080bcca69341031e1ac54bc5c4c60213)) 

  ## Summary
  
  - ironrdp-rdpdr only decodes what a client receives from a server (6
  PacketIds) and only encodes what a client sends to a server. A server
  needs the opposite. This makes the PDU layer bidirectional: decode()
  added to every response type the client currently only encodes, and
  encode() added to every request type the client currently only decodes.
  - CoreCapability::decode already branched on packet_id for both
  directions and just needed the client-capability arm wired into
  RdpdrPdu::decode_body, alongside three new decodes (ClientNameRequest,
  ClientDeviceListAnnounce, ClientDeviceListRemove).
  - CoreDeviceIoCompletion is deliberately not wired into the automatic
  RdpdrPdu::decode dispatch. Fourteen different response variants share
  that one PacketId (see the header() method's match), and nothing on the
  wire disambiguates which one a given completion answers. That requires
  knowing the MajorFunction (and, for DirectoryControl, MinorFunction) of
  the original request, which only a future IRP-tracking layer will have.
  Added RdpdrPdu::decode_io_completion as an explicit entry point for that
  layer to call once it exists, taking the major/minor function and, for
  the three completions whose body layout depends on it, the
  FileInformationClass the original request asked for.
  - DeviceControlResponse's output_buffer is a Box<dyn rpce::Encode> on
  the encode side, since it carries IOCTL-specific NDR content only the
  issuer of the original request can interpret. That can't be decoded back
  into a concrete type. Decode reads it as opaque bytes through a private
  RawOutputBuffer implementing the same trait, so the decoded value has
  the same shape without pretending to know a structure it can't actually
  know.
  - Filled two response-decode gaps that were there before this PR touched
  the file, found while checking the new decode paths against MS-RDPEFS
  directly: FileStandardInformation and FileAttributeTagInformation are
  two of the exact three classes 2.2.3.3.8 allows for a Query Information
  response (the third, Basic, already round-tripped) and had encode with
  no decode. FileDirectoryInformation, FileFullDirectoryInformation,
  FileBothDirectoryInformation, and FileNamesInformation are the complete
  set 2.2.3.3.10 allows for a Query Directory response and were entirely
  unwired in FileInformationClass::decode, so directory listing had no
  working decode path at all.
  - VersionAndIdPdu::decode always tagged PacketId::CoreClientidConfirm as
  ServerClientIdConfirm. Per MS-RDPEFS 2.2.1.1, that PacketId is shared
  between Server Client ID Confirm (2.2.2.6) and Client Announce Reply
  (2.2.2.3): correct for a client, which never decodes its own reply, but
  wrong for a server decoding the client's reply. Added
  decode_client_announce_reply as an explicit entry point for that case,
  mirroring decode_io_completion's precedent for the same class of
  PacketId ambiguity, and left decode() itself untouched.
  - DeviceControlRequest<T> had decode and decode_with_input_buffer but no
  encode, the one server-sent I/O request missing it. Adding encode
  required tightening IoCtlCode to also require Into<u32>, which surfaced
  that ScardIoCtlCode (the trait's only other implementor) was missing
  that conversion too.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Round-trip
  tests added for every new decode()/encode() pair in
  ironrdp-testsuite-core/tests/rdpdr/mod.rs, including one per Query
  Information and Query Directory class, plus the two additions above.
  
  ## Notes
  
  PDU-codec layer only. The RdpdrServer state machine, IRP tracking, and
  ironrdp-server integration (a factory trait plus attach_channels wiring,
  the same shape as the recently landed RdpeiServerFactory) are a
  follow-up PR once this lands, so the server-relevant surface can be
  reviewed in a size a reviewer can actually hold in their head at once.

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

- Redirect dynamic drives ([#1780](https://github.com/Devolutions/IronRDP/issues/1780)) ([5e05637f06](https://github.com/Devolutions/IronRDP/commit/5e05637f0659d460d6e02b5567bb728f1d04585a)) 

  Keep drive capability active for hotplug-only sessions and route ActiveX
  drive rescans and selection changes through the live RDPDR channel.
  Preserve stable drive IDs, defer removals until server acknowledgement,
  and keep generic devices, printers, and ports explicitly unsupported.
  
  Smartcard redirection remains independent.

- Add RdpdrServer orchestration on top of the bidirectional PDU layer ([#1783](https://github.com/Devolutions/IronRDP/issues/1783)) ([fb5f662af9](https://github.com/Devolutions/IronRDP/commit/fb5f662af9d8f3ee39429b9da00f84c5cad8582a)) 

  ## Depends on #1779
  
  This branch is stacked on `feat/rdpdr-server-pdu-decode` ([#1779](https://github.com/Devolutions/IronRDP/issues/1779)) and
  only makes sense once that lands: `RdpdrServer` is built entirely on the
  decode/encode surface #1779 adds. #1779's own Notes section already
  flags this as the intended follow-up.
  
  ## Summary
  
  - `RdpdrServer` is a state machine (`Initializing`, `AwaitingAnnounce`,
  `CapabilityExchange`, `Active`) driven by `SvcProcessor::process`,
  following the handshake sequence MS-RDPEFS 3.1.5/3.2.5 describe.
  `CapabilityExchange` and `Active` both accept `CoreDevicelistAnnounce`
  since real clients don't always send it strictly during capability
  exchange. A client re-sending its Announce Reply while already `Active`
  re-initializes (issue #195's duplicate-init case) rather than erroring:
  devices and in-flight IRPs clear, the backend sees a fresh
  `on_device_announce` batch.
  - Outstanding drive I/O is tracked by `CompletionId` in an `IrpTracker`,
  mirroring `ironrdp-rdpeusb`'s `RequestIdAllocator`/`pending_io`
  precedent. Each `PendingIrp` variant carries exactly the context
  `RdpdrPdu::decode_io_completion` needs (`MajorFunction`,
  `MinorFunction`, and the file/filesystem information class where the
  response layout depends on it). An unrecognized `CompletionId` is logged
  and ignored, not treated as an error.
  - `RdpdrServerBackend` has one completion callback per `PendingIrp`
  variant (fourteen, matching `MajorFunction` exactly except the
  deliberately unsupported `SetVolumeInformation`) plus
  `on_device_announce`, `on_device_remove`, `on_client_name`.
  `NoopRdpdrServerBackend` accepts every device and no-ops the rest.
  - Fourteen `drive_*` methods build and send each request type. Thirteen
  share a `DriveRequestBody` trait and a `send_device_io_request` helper;
  `device_control` needs its own wrapper since `DeviceControlRequest`'s
  encode doesn't include the input buffer that follows it on the wire,
  matching its decode-side counterpart.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Full
  handshake, per-drive-method completion round trips (all fourteen),
  device accept/reject, duplicate-init, and orphaned-completion-id tests
  added in `ironrdp-testsuite-core/tests/rdpdr/mod.rs`.
  
  ## Notes
  
  `ironrdp-server` integration (a factory trait plus `attach_channels`
  wiring, matching `RdpeiServerFactory`'s recently landed shape) is a
  further follow-up once this lands.

- Wire RdpdrServer into ironrdp-server ([#1784](https://github.com/Devolutions/IronRDP/issues/1784)) ([695af5a6e8](https://github.com/Devolutions/IronRDP/commit/695af5a6e814badb3daa60184a8d2048d0e66716)) 

  ## Depends on #1783
  
  Stacked on `feat/rdpdr-server-core` ([#1783](https://github.com/Devolutions/IronRDP/issues/1783)), which itself depends on
  #1779. Neither PDU codec bidirectionality nor RdpdrServer's
  orchestration is reachable from ironrdp-server without this.
  
  ## Summary
  
  - Wires RdpdrServer into ironrdp-server, mirroring SoundServerFactory
  and RdpsndServer exactly. RDPDR is a static virtual channel with the
  same build-a-backend-then-attach shape as audio, not a dynamic channel
  like EGFX and not a combined backend-plus-factory like clipboard.
  - RdpdrServerFactory extends the ServerEventSender supertrait rather
  than inlining a set_sender method, matching the live convention
  SoundServerFactory and CliprdrServerFactory already use. build_backend
  is named to match SoundServerFactory::build_backend.
  - Server-initiated drive I/O needs the same async relay every other
  channel uses. Added RdpdrServerMessage, one variant per RdpdrServer
  drive_* method, and a ServerEvent::Rdpdr(RdpdrServerMessage) arm in
  dispatch_server_events that looks up the live RdpdrServer instance,
  calls the matching drive_* method, and writes the encoded result, the
  same shape as the existing Rdpsnd arm.
  - Adding a fourteen-variant enum to ServerEvent pushed
  AutoReconnectCookieHandle::set's Result past clippy's result_large_err
  threshold, a method this change otherwise has nothing to do with.
  Suppressed locally with an explained reason rather than boxing the Rdpdr
  payload, which would have changed ServerEvent's public shape for a size
  concern from one unrelated method.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Added an
  exhaustive-match test over every RdpdrServerMessage variant in
  ironrdp-testsuite-core/tests/server/rdpdr.rs (no wildcard arm), so a
  future variant added to one side of the dispatch without the other fails
  to compile rather than silently falling through.
  
  ## Notes
  
  This closes out the RDPDR server-side contribution's three-part sequence
  (PDU codec, RdpdrServer orchestration, ironrdp-server wiring).

- Redirect default Windows printer ([#1876](https://github.com/Devolutions/IronRDP/issues/1876)) ([eaadd650c9](https://github.com/Devolutions/IronRDP/commit/eaadd650c9e2be98b57274b72095bf41da6ac3dd)) 

  Expose the preconnect RedirectPrinters setting and carry printer
  metadata through the RDPDR client factory.
  
  Discover the current Windows default queue, negotiate its driver
  metadata, and stream bounded RAW jobs on a module-pinned worker. Ports,
  generic PnP/USB devices, multiple printers, and Easy Print/XPS remain
  unsupported.

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

- Consume query information body ([#1810](https://github.com/Devolutions/IronRDP/issues/1810)) ([476e8a174d](https://github.com/Devolutions/IronRDP/commit/476e8a174d6a61b7dbadbf4fe0e3e2290fbfc491)) 

  DR_DRIVE_QUERY_INFORMATION_REQ previously decoded only the
  FsInformationClass field and ignored Length, Padding, and QueryBuffer,
  leaving those bytes unconsumed on the wire cursor. Servers that send a
  non-empty QueryBuffer (per MS-RDPEFS 2.2.3.3.8) desynchronized
  subsequent PDU parsing.
  
  Decode and encode the full fixed part (FsInformationClass, Length,
  24-byte Padding) and advance the cursor past the Length-bounded
  QueryBuffer on decode. Encode continues to emit an empty QueryBuffer
  since this representation only retains the information class.

- Defer User Logged On that arrives before Client ID Confirm ([#2079](https://github.com/Devolutions/IronRDP/issues/2079)) ([e88e3c51f9](https://github.com/Devolutions/IronRDP/commit/e88e3c51f9c3120185fac101206041989e3292e0)) 

  After the user logs in, Windows re-announces the RDPDR channel and sends
  Server User Logged On before the new Server Core Capability Request and
  Server Client ID Confirm. `Rdpdr` treated that order as a protocol
  error, so a connection to a Windows 11 host ended a few seconds after
  login with:
  
  ```
  PDU error: [ironrdp_rdpdr::Rdpdr::handle_user_logged_on] other (received RDPDR user logged on before client ID confirmation)
  ```
  
  Observed sequence (`ironrdp_rdpdr=debug`):
  
  ```
  ServerAnnounceRequest   client_id: 3
  ServerCoreCapabilityRequest
  ServerClientIdConfirm   client_id: 3
  ServerAnnounceRequest   client_id: 2
  UserLoggedon                          <- previously fatal
  ServerCoreCapabilityRequest
  ServerClientIdConfirm   client_id: 2
  ```
  
  `Rdpdr` now remembers a User Logged On that arrives before the Client ID
  Confirm and handles it right after the confirmation, so the backend's
  `handle_user_logged_on` and the post-logon device announcement still run
  once the client ID is known. A new Server Announce clears the pending
  flag. FreeRDP accepts the same order: its state check allows
  `PAKID_CORE_USER_LOGGEDON` before `PAKID_CORE_CLIENTID_CONFIRM`.

- Complete unsupported drive IRPs instead of failing ([#2174](https://github.com/Devolutions/IronRDP/issues/2174)) ([0f9154d212](https://github.com/Devolutions/IronRDP/commit/0f9154d2128f8c65f4d51f6b7eb7cb1d66a12329)) 

  ## Problem
  
  A well-formed drive I/O request that IronRDP does not implement was a
  decode error in the RDPDR processor. That error goes through
  `svc.process(...).map_err(SessionError::pdu)?` in `ironrdp-session`, so
  the whole RDP session ended. The requests affected were:
  
  - `IRP_MJ_SET_VOLUME_INFORMATION`: `ServerDriveIoRequest::decode`
  returns `unsupported_value_err!`.
  - Set information, query volume information and query directory requests
  whose information class is not one of the classes the decoders accept.
  - `IRP_MJ_DIRECTORY_CONTROL` with a minor function other than
  QUERY_DIRECTORY or NOTIFY_CHANGE_DIRECTORY.
  
  FreeRDP's drive channel completes these requests with an error status,
  and the session continues.
  
  ## Fix
  
  `Rdpdr` now checks filesystem IRPs before decoding them. When a request
  is one of the unsupported kinds above, the processor completes it itself
  with the request's DeviceId and CompletionId, and the backend never sees
  it. Each reply has the layout the server expects for that request type:
  
  | Request | IoStatus | Reply body |
  | --- | --- | --- |
  | Set volume information | `STATUS_NOT_SUPPORTED` | Length echoed back
  (MS-RDPEFS 2.2.3.4.7 says it MUST match the request) |
  | Set information, unsupported class | `STATUS_NOT_SUPPORTED` | Length
  echoed back |
  | Query volume information, unsupported class | `STATUS_NOT_SUPPORTED` |
  Empty buffer |
  | Query directory, unsupported class | `STATUS_NOT_SUPPORTED` | Empty
  buffer and the 1-byte padding |
  | Directory control, undescribed minor function | `STATUS_UNSUCCESSFUL`
  | None |
  
  The directory-control case follows MS-RDPEFS 3.1.5.2: a MajorFunction or
  MinorFunction the protocol does not describe "SHOULD be completed" with
  `STATUS_UNSUCCESSFUL`. FreeRDP uses `STATUS_NOT_SUPPORTED` there.
  
  Truncated requests are still decode errors.
  
  The lists of accepted information classes now live in crate-private
  `is_valid_class` helpers, which the decoders and the new check share. No
  public API changes.
  
  Query information requests already accepted any class and passed it to
  the backend, which answers with a status. The native backends already
  reply `STATUS_NOT_SUPPORTED` for operations they do not implement.
  
  ## Tests
  
  `ironrdp-testsuite-core` has six new tests in `tests/rdpdr/mod.rs`. Five
  check the status, device and completion IDs, and reply layout for each
  case in the table. The sixth checks that truncated requests still fail.
  
  Ran `cargo xtask check fmt -v`, `cargo xtask check lints -v`, the
  `ironrdp-rdpdr` and `ironrdp-rdpdr-native` tests, and the rdpdr tests in
  `ironrdp-testsuite-core`. All pass.
  
  ## Not covered
  
  Two more requests can still end the session the same way. They are left
  for separate changes:
  
  - A MajorFunction value the protocol does not define fails in
  `DeviceIoRequest::decode`, before dispatch. MS-RDPEFS 3.1.5.2 asks for a
  `STATUS_UNSUCCESSFUL` completion.
  - Smartcard IRPs: a request that is not a device control is decoded as
  one anyway, and an unknown smartcard IOCTL code is a decode error.
  FreeRDP answers both with `STATUS_NOT_SUPPORTED`.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

- Serve IRPs for devices awaiting announcement response ([#2191](https://github.com/Devolutions/IronRDP/issues/2191)) ([29fc63dae4](https://github.com/Devolutions/IronRDP/commit/29fc63dae489548cc3b4c9e0bbb97b196cec56ef)) 

  Windows can send the first IRP for a redirected device just before the
  `ServerDeviceAnnounceResponse` that accepts it. `ironrdp-rdpdr` ignored
  such IRPs without completing them, so the server waited about 30 seconds
  and then retried.
  
  This showed up in a live test of drive redirection ([#2178](https://github.com/Devolutions/IronRDP/issues/2178)) against a
  Windows Server session, after a reconnect. The log shows the order:
  
  ```
  DeviceIoRequest { device_id: 3, major_function: Create }
  WARN Ignoring filesystem IRP before device announcement confirmation device_id=3
  ServerDeviceAnnounceResponse { device_id: 3, result_code: STATUS_SUCCESS }   (same millisecond)
  ... 30 s later ...
  DeviceIoRequest { device_id: 3, major_function: Create }
  ```
  
  ## Change
  
  A device awaiting its announcement response now counts as announced, for
  both filesystem and printer IRPs (`Rdpdr::device_is_announced`). FreeRDP
  behaves the same way: it serves every registered device and only
  unregisters one when the server rejects it.
  
  - If the server later rejects the device, the existing rejection path
  still removes the drive from the backend, which cancels its pending
  IRPs.
  - A printer `Create` before acceptance is completed with
  `STATUS_UNSUCCESSFUL` in the channel and does not reach the backend. It
  would start a native print job that a rejection could leave allocated,
  because IRPs for rejected devices, including the close, are ignored.
  Other early printer IRPs complete without allocating.
  - IRPs for devices that were never announced, or were rejected, are
  still ignored, as MS-RDPEFS recommends.
  
  ## Testing
  
  - New `ironrdp-rdpdr` tests: a drive IRP before the response reaches the
  backend; an early IRP followed by a rejection removes the drive. New
  core testsuite tests: a printer IRP before the response is completed, an
  early printer `Create` fails without reaching the backend, and a
  `Create` after acceptance reaches it. All three fail without the fix.
  - `cargo xtask check fmt -v` and `cargo xtask check lints -v` pass, as
  do the tests in `ironrdp-rdpdr`, `ironrdp-rdpdr-native`,
  `ironrdp-testsuite-core` and `ironrdp-testsuite-extra`.
  - The live reconnect has not been rerun with this fix.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.6.0...ironrdp-rdpdr-v0.7.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.5.0...ironrdp-rdpdr-v0.6.0)] - 2026-05-27

### <!-- 1 -->Features

- Notify RdpdrBackend of 'User Logged On' Messages ([#1211](https://github.com/Devolutions/IronRDP/issues/1211)) ([1a09dbaca9](https://github.com/Devolutions/IronRDP/commit/1a09dbaca9dd5d35025ee50aaa645100222be189)) 

- Add Web RDPDR virtual printer support ([#1230](https://github.com/Devolutions/IronRDP/issues/1230)) ([14b1cef9cb](https://github.com/Devolutions/IronRDP/commit/14b1cef9cbbd0d8ef5e1fc8c73a3003a5e9f9bc2)) 

  Adds RDPDR virtual printer redirection for web sessions, enabling the web client to announce a redirected printer, receive server print jobs over RDPDR, and deliver completed PostScript jobs to a browser callback.

### <!-- 4 -->Bug Fixes

- Model CreateDisposition as enum instead of bitflags ([#1145](https://github.com/Devolutions/IronRDP/issues/1145)) ([c4f87aa417](https://github.com/Devolutions/IronRDP/commit/c4f87aa417e83c9cf6d1550c877ea3facb2f9a59)) 

  CreateDisposition values (FILE_SUPERSEDE through FILE_OVERWRITE_IF) are
  mutually exclusive integers 0 through 5, not combinable bit flags.
  Modeling them with the bitflags macro causes subtle correctness issues.

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd)) 

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.4.1...ironrdp-rdpdr-v0.5.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- Fix incorrect padding when parsing NDR strings ([#1015](https://github.com/Devolutions/IronRDP/issues/1015)) ([a0a3e750c9](https://github.com/Devolutions/IronRDP/commit/a0a3e750c9e4ee9c73b957fbcb26dbc59e57d07d)) 

  When parsing Network Data Representation (NDR) messages, we're supposed
  to account for padding at the end of strings to remain aligned on a
  4-byte boundary. The existing code doesn't seem to cover all cases, and
  the resulting misalignment causes misleading errors when processing the
  rest of the message.

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.4.0...ironrdp-rdpdr-v0.4.1)] - 2025-09-04

### <!-- 1 -->Features

- Support device removal (#947) ([50574c570f](https://github.com/Devolutions/IronRDP/commit/50574c570f6e44d264153337e5f87a5313f190e6)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.2.0...ironrdp-rdpdr-v0.3.0)] - 2025-05-27

### <!-- 1 -->Features

- Add USER_LOGGEDON flag support ([5e78f91713](https://github.com/Devolutions/IronRDP/commit/5e78f917132a174bdd5d8711beb1744de1bd265a)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.3...ironrdp-rdpdr-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.2...ironrdp-rdpdr-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.1...ironrdp-rdpdr-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.0...ironrdp-rdpdr-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
