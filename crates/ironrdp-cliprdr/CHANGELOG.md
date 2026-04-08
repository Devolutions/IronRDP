# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

### <!-- 1 -->Features

- [**breaking**] Add clipboard file transfer support per MS-RDPECLIP

  Implements end-to-end clipboard file transfer (upload and download) across the
  CLIPRDR channel. Key changes:

  - Automatic clipboard locking: when `FileGroupDescriptorW` is detected in a
    FormatList, the processor automatically sends Lock PDUs and manages the lock
    lifecycle (expiry, cleanup, Unlock PDUs) internally.
  - New `CliprdrBackend` methods with default implementations:
    - `on_remote_file_list()` - called when remote announces files
    - `on_file_contents_request()` - called when remote requests file data
    - `on_outgoing_locks_cleared()` - called when locks are released
    - `on_outgoing_locks_expired()` - called when locks expire
    - `now_ms()` / `elapsed_ms()` - time source for timeout tracking
  - New `drive_timeouts()` method for callers to invoke periodically to clean up
    stale locks and pending requests.
  - Comprehensive path sanitization to protect against path traversal attacks.

- [**breaking**] Remove `ClipboardMessage::SendLockClipboard` and `SendUnlockClipboard` variants

  Lock/unlock is now managed internally by the `Cliprdr` processor. Backends no
  longer need to handle these messages. Remove any code that matches on these
  variants.

- [**breaking**] Rename `FileContentsFlags::DATA` to `FileContentsFlags::RANGE`

  Aligns with MS-RDPECLIP 2.2.5.3 terminology where this flag indicates a
  "range" request for file data bytes. Replace `FileContentsFlags::DATA` with
  `FileContentsFlags::RANGE` in your code.

- [**breaking**] Change `FileContentsRequest::index` type from `u32` to `i32`

  Per MS-RDPECLIP 2.2.5.3, the `lindex` field is a signed 32-bit integer.
  This corrects the spec compliance. Update code to use `i32` for the index field.

- [**breaking**] Make `FileDescriptor` `#[non_exhaustive]` and add `relative_path` field

  The `FileDescriptor` struct is now marked `#[non_exhaustive]` to allow future
  field additions without breaking changes. A new `relative_path: Option<String>`
  field has been added to support directory structure in file transfers.

  **Migration:** Use the builder pattern instead of struct literals:
  ```rust
  // Before (no longer compiles)
  let desc = FileDescriptor { name: "file.txt".into(), file_size: Some(1024), ... };

  // After
  let desc = FileDescriptor::new("file.txt").with_file_size(1024);
  ```

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
