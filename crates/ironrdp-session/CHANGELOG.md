# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.8.0...ironrdp-session-v0.9.0)] - 2026-03-01

### <!-- 0 -->Security

- Dispatch multitransport PDUs on IO channel ([#1096](https://github.com/Devolutions/IronRDP/issues/1096)) ([7853e3cc6f](https://github.com/Devolutions/IronRDP/commit/7853e3cc6f26acaf3da000c6177ca3cef6ef85fd)) 

  `decode_io_channel()` assumes all IO channel PDUs begin with
  a`ShareControlHeader`. Multitransport Request PDUs use a
  `BasicSecurityHeader` with `SEC_TRANSPORT_REQ` instead ([MS-RDPBCGR]
  2.2.15.1).
  
  This adds a peek-based dispatch: check the first `u16`
  for`TRANSPORT_REQ`, decode as `MultitransportRequestPdu` if set,
  otherwise fall through to the existing `decode_share_control()` path
  unchanged.
  
  The new variant is propagated through `ProcessorOutput` and
  'ActiveStageOutput` so applications can handle multitransport requests.
  Client and web consumers log the request (no UDP transport yet).

### <!-- 1 -->Features

- Add bulk compression and wire negotiation ([ebf5da5f33](https://github.com/Devolutions/IronRDP/commit/ebf5da5f3380a3355f6c95814d669f8190425ded)) 

  - add ironrdp-bulk crate with MPPC/NCRUSH/XCRUSH, bitstream, benches, and metrics
  - advertise compression in Client Info and plumb compression_type through connector
  - decode compressed FastPath/ShareData updates using BulkCompressor
  - update CLI to numeric compression flags (enabled by default, level 0-3)
  - extend screenshot example with compression options and negotiated logging
  - refresh tests, FFI/web configs, typos, and Cargo.lock

### <!-- 4 -->Bug Fixes

- Make fields of Error private ([#1074](https://github.com/Devolutions/IronRDP/issues/1074)) ([e51ed236ce](https://github.com/Devolutions/IronRDP/commit/e51ed236ce5d55dc1a4bc5f5809fd106bdd2e834)) 

- Fix pixel format handling in bitmap decoders ([#1101](https://github.com/Devolutions/IronRDP/issues/1101)) ([75863245ab](https://github.com/Devolutions/IronRDP/commit/75863245ab376f15e35c00df434860c93b123633)) 

- Handle row padding in uncompressed bitmap updates ([4262ae75ff](https://github.com/Devolutions/IronRDP/commit/4262ae75ffa5cb1fabb4ca07d598e33d855e8fdd)) 

  Uncompressed bitmap data has rows padded to 4-byte boundaries per
  [MS-RDPBCGR] 2.2.9.1.1.3.1.2.2, but the bitmap apply functions
  expect tightly packed pixel data. Strip the per-row padding before
  passing raw bitmap data to the apply functions.
  
  This fixes garbled bitmap rendering when connecting to servers that
  send uncompressed bitmaps with non-aligned row widths, such as XRDP
  at 16 bpp.



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.7.0...ironrdp-session-v0.8.0)] - 2025-12-18


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.5.0...ironrdp-session-v0.6.0)] - 2025-08-29

### <!-- 1 -->Features

- Add QOI image codec ([613fd51f26](https://github.com/Devolutions/IronRDP/commit/613fd51f26315d8212662c46f8e625c541e4bb59)) 

  The Quite OK Image format ([1]) losslessly compresses images to a similar size
  of PNG, while offering 20x-50x faster encoding and 3x-4x faster decoding.

- Add QOIZ image codec ([87df67fdc7](https://github.com/Devolutions/IronRDP/commit/87df67fdc76ff4f39d4b83521e34bf3b5e2e73bb)) 

  Add a new QOIZ codec for SetSurface command. The PDU data contains the same
  data as the QOI codec, with zstd compression.

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.4.0...ironrdp-session-v0.4.1)] - 2025-06-27

### <!-- 1 -->Features

- More functions on `ActiveStage` (#791) ([5482365655](https://github.com/Devolutions/IronRDP/commit/5482365655e5c171cd967eda401b01161a9f6602)) 
  - `get_dvc_by_channel_id`
  - `encode_dvc_messages`


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.3.0...ironrdp-session-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- [**breaking**] Make DecodedImage Send ([45f66117ba](https://github.com/Devolutions/IronRDP/commit/45f66117ba05170d95b21ec7d97017b44b954f28)) 

- Add DecodeImage helpers ([cd7a60ba45](https://github.com/Devolutions/IronRDP/commit/cd7a60ba45a0241be4ecf3860ec4f82b431a7ce2)) 

### <!-- 4 -->Bug Fixes

- Update rectangle when applying None codecs updates (#728) ([a50cd643dc](https://github.com/Devolutions/IronRDP/commit/a50cd643dce9621f314231b7598d2fd31e4718c6)) 

- Return the correct updated region ([7507a152f1](https://github.com/Devolutions/IronRDP/commit/7507a152f14db594e4067bbc01e243cfba77770f)) 

  "update_rectangle" is set to empty(). The surface updates are then added
  by "union". But a union with an empty rectangle at (0,0) is still a
  rectangle at (0,0). We end up with big region updates rooted at (0,0)...

- Decrease verbosity of Rfx frame_index ([b31b99eafb](https://github.com/Devolutions/IronRDP/commit/b31b99eafb0aac2a5e5a610af21a4027ae5cd698)) 

- Decrease verbosity of FastPath header ([f9b6992e74](https://github.com/Devolutions/IronRDP/commit/f9b6992e74abb929f3001e76abaff5d7215e1cb4)) 


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.3...ironrdp-session-v0.3.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.2...ironrdp-session-v0.2.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.1...ironrdp-session-v0.2.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.0...ironrdp-session-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
