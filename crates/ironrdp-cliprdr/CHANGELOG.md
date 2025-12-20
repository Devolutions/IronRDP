# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-v0.4.0...ironrdp-cliprdr-v0.5.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- [**breaking**] Receiving a TemporaryDirectory PDU should not fail the svc ([#1031](https://github.com/Devolutions/IronRDP/issues/1031)) ([f2326ef046](https://github.com/Devolutions/IronRDP/commit/f2326ef046cc81fb0e8985f03382859085882e86)) 

  - Fixes the Cliprdr `SvcProcessor` impl to support handling a `TemporaryDirectory` Clipboard PDU.
  - Removes `ClipboardError::UnimplementedPdu` since it is no longer used

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
