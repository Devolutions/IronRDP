# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.7.0...ironrdp-rdpdr-v0.8.0)] - 2026-09-01

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

### <!-- 4 -->Bug Fixes

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
