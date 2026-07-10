# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.6.0...ironrdp-cliprdr-v0.7.0)] - 2026-07-10

### <!-- 1 -->Features

- Dispatch initiate_file_copy via ClipboardMessage ([#1388](https://github.com/Devolutions/IronRDP/issues/1388)) ([b6325f9ea6](https://github.com/Devolutions/IronRDP/commit/b6325f9ea6900a84643b4415f9ebc7b1010cf3cd)) 

  Extends the CLIPRDR backend-facing API to properly support offering clipboard file lists (so later FileContentsRequests can be serviced) by introducing ClipboardMessage::SendInitiateFileCopy(Vec<FileDescriptor>) and wiring it through the in-tree ClipboardMessage dispatchers.

### <!-- 4 -->Bug Fixes

- Release outgoing locks before initiating a file copy ([#1375](https://github.com/Devolutions/IronRDP/issues/1375)) ([5d534f10a6](https://github.com/Devolutions/IronRDP/commit/5d534f10a6f62ac7a860521b4e95c8c47b754612)) 

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.5.0...ironrdp-cliprdr-v0.6.0)] - 2026-05-27

### <!-- 1 -->Features

- [**breaking**] Implement clipboard file transfer support ([#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9))

  Add end-to-end clipboard file transfer (upload and download) across the
  CLIPRDR channel per MS-RDPECLIP. Automatic clipboard locking: when
  `FileGroupDescriptorW` is detected in a FormatList, the processor
  automatically sends Lock PDUs and manages the lock lifecycle (expiry,
  cleanup, Unlock PDUs) internally.

  New `CliprdrBackend` methods (with default implementations):
  `on_remote_file_list()`, `on_file_contents_request()`,
  `on_outgoing_locks_cleared()`, `on_outgoing_locks_expired()`, and
  `now_ms()` / `elapsed_ms()` for timeout tracking. New `drive_timeouts()`
  method for callers to invoke periodically. Comprehensive path
  sanitization to protect against path traversal attacks.

  Breaking changes folded in: removed `ClipboardMessage::SendLockClipboard`
  and `SendUnlockClipboard` variants (lock/unlock is now managed
  internally); renamed `FileContentsFlags::DATA` to `RANGE` (matches
  MS-RDPECLIP 2.2.5.3 terminology); changed `FileContentsRequest::index`
  from `u32` to `i32` (per spec); made `FileDescriptor` `#[non_exhaustive]`
  and added `relative_path: Option<String>` field (use the builder pattern
  instead of struct literals).

- Add clipboard data locking methods ([#1064](https://github.com/Devolutions/IronRDP/issues/1064)) ([58c3df84bb](https://github.com/Devolutions/IronRDP/commit/58c3df84bb9cafc8669315834cead35a71483c34))

  Per MS-RDPECLIP sections 2.2.4.6 and 2.2.4.7, the local
  clipboard owner can lock shared clipboard data before requesting file
  contents, ensuring data stability during multi-request transfers.

- Add request_file_contents method ([#1065](https://github.com/Devolutions/IronRDP/issues/1065)) ([c30fc35a28](https://github.com/Devolutions/IronRDP/commit/c30fc35a28d6218603c1662e98e8b3053bea3aa5))

  Per MS-RDPECLIP section 2.2.5.3, this adds support
  for sending File Contents Request PDUs to retrieve remote file data
  during paste operations.

- Add SendFileContentsResponse message variant ([#1066](https://github.com/Devolutions/IronRDP/issues/1066)) ([25f81337aa](https://github.com/Devolutions/IronRDP/commit/25f81337aa494af9a21f55f12ec27fd946465cbe))

  Adds `SendFileContentsResponse` to `ClipboardMessage`, allowing
  clipboard backends to signal when file data is ready to be sent via
  `submit_file_contents()`.

- Always set FD_PROGRESSUI in FileDescriptor::encode ([#1299](https://github.com/Devolutions/IronRDP/issues/1299)) ([7e0bfd3c55](https://github.com/Devolutions/IronRDP/commit/7e0bfd3c550135a3c9c85cb66a478ce41c8641d9))

- Advertise Preferred DropEffect alongside FileGroupDescriptorW ([#1301](https://github.com/Devolutions/IronRDP/issues/1301)) ([5375bbb9dd](https://github.com/Devolutions/IronRDP/commit/5375bbb9ddb8b853973d050fa2efd0ed217ac17b))

  `initiate_file_copy` now advertises **both** `FileGroupDescriptorW` and
  `Preferred DropEffect` (`CFSTR_PREFERREDDROPEFFECT`) in the FormatList,
  and `handle_format_data_request` short-circuits a request for the latter
  with `DROPEFFECT_COPY` (0x00000001 LE).

- [**breaking**] Add CliprdrBackend::on_format_list_response(ok) hook ([#1300](https://github.com/Devolutions/IronRDP/issues/1300)) ([a4bc475360](https://github.com/Devolutions/IronRDP/commit/a4bc4753607d87ef0989d9df16a31cd22e7c7fde))

### <!-- 4 -->Bug Fixes

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd))

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576))


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.4.0...ironrdp-cliprdr-v0.5.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- Fixes the Cliprdr `SvcProcessor` impl to support handling a `TemporaryDirectory` Clipboard PDU ([#1031](https://github.com/Devolutions/IronRDP/issues/1031)) ([f2326ef046](https://github.com/Devolutions/IronRDP/commit/f2326ef046cc81fb0e8985f03382859085882e86)) 

- Allow servers to announce clipboard ownership ([#1053](https://github.com/Devolutions/IronRDP/issues/1053)) ([d587b0c4c1](https://github.com/Devolutions/IronRDP/commit/d587b0c4c114c49d30f52859f43b22f829456a01)) 

  Servers can now send Format List PDU via initiate_copy() regardless of
  internal state. The existing state machine was designed for clients
  where clipboard initialization must complete before announcing
  ownership.
  
  MS-RDPECLIP Section 2.2.3.1 specifies that Format List PDU is sent by
  either client or server when the local clipboard is updated. Servers
  should be able to announce clipboard changes immediately after channel
  negotiation.
  
  This change enables RDP servers to properly announce clipboard ownership
  by bypassing the Initialization/Ready state check when R::is_server() is
  true. Client behavior remains unchanged.

- [**breaking**] Removed the `PackedMetafile::data()` method in favor of making the `PackedMetafile::data` field public.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.3.0...ironrdp-cliprdr-v0.4.0)] - 2025-08-29

### <!-- 4 -->Bug Fixes

- [**breaking**] Remove the `on_format_list_received` callback (#935) ([5b948e2161](https://github.com/Devolutions/IronRDP/commit/5b948e2161b08b13d32bdbb480b26c8fa44d42f7)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.2.0...ironrdp-cliprdr-v0.3.0)] - 2025-05-27

### <!-- 1 -->Features

- [**breaking**] Add on_ready() callback (#729) ([4e581e0f47](https://github.com/Devolutions/IronRDP/commit/4e581e0f47593097c16f2dde43cd0ff0976fe73e)) 

  Give a hint to the backend when the channel is actually connected &
  ready to process messages.


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.1.3...ironrdp-cliprdr-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.1.2...ironrdp-cliprdr-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.1.1...ironrdp-cliprdr-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.1.0...ironrdp-cliprdr-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
