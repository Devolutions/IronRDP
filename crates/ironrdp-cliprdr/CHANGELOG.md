# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.5.0...ironrdp-cliprdr-v0.6.0)] - 2026-03-01

### <!-- 1 -->Features

- Add clipboard data locking methods ([#1064](https://github.com/Devolutions/IronRDP/issues/1064)) ([58c3df84bb](https://github.com/Devolutions/IronRDP/commit/58c3df84bb9cafc8669315834cead35a71483c34)) 

  Per [MS-RDPECLIP sections 2.2.4.6 and 2.2.4.7][lock-spec], the Local
  Clipboard
  Owner may lock the Shared Clipboard Owner's clipboard data before
  requesting
  file contents to ensure data stability during multi-request transfers.
  
  This enables server implementations to safely request file data from
  clients
  when handling clipboard paste operations.
  
  ---------

- Add request_file_contents method ([#1065](https://github.com/Devolutions/IronRDP/issues/1065)) ([c30fc35a28](https://github.com/Devolutions/IronRDP/commit/c30fc35a28d6218603c1662e98e8b3053bea3aa5)) 

  Per [MS-RDPECLIP section 2.2.5.3][file-contents-spec], the Local
  Clipboard Owner
  sends File Contents Request PDU to retrieve file data from the Shared
  Clipboard
  Owner during paste operations.
  
  This enables server implementations to request file contents from
  clients,
  completing the bidirectional file transfer capability.

- Add SendFileContentsResponse message variant ([#1066](https://github.com/Devolutions/IronRDP/issues/1066)) ([25f81337aa](https://github.com/Devolutions/IronRDP/commit/25f81337aa494af9a21f55f12ec27fd946465cbe)) 

  Adds `SendFileContentsResponse` to `ClipboardMessage` enum, enabling
  clipboard
  backends to signal when file data is ready to send via
  `submit_file_contents()`.
  
  This provides the message-based interface pattern used consistently by
  server
  implementations for clipboard operations.



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
