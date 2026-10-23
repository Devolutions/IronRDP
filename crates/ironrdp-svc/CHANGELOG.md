# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.8.0...ironrdp-svc-v0.9.0)] - 2026-10-23

### <!-- 1 -->Features

- Add runtime-defined static channels: `StaticChannelSet::insert_dynamic` returns a `StaticChannelKey`, used by key- and name-based accessors such as `get_by_key`, `attach_channel_id_by_key`, `get_channel_id_by_channel_name` and `iter_by_key`, and the set gains `len` and `is_empty` ([#1517](https://github.com/Devolutions/IronRDP/issues/1517)) ([8b4c483ba0](https://github.com/Devolutions/IronRDP/commit/8b4c483ba0c900a8de0b2718347754f56dd363ba))

- `StaticChannelSet` holds at most `MAX_STATIC_CHANNELS` (31) channels, and `insert` no longer adds a new channel past that limit ([#1517](https://github.com/Devolutions/IronRDP/issues/1517)) ([8b4c483ba0](https://github.com/Devolutions/IronRDP/commit/8b4c483ba0c900a8de0b2718347754f56dd363ba))

- Add the provided `SvcProcessor::channel_options` method to set the channel options sent in the GCC Client Network Data, readable through `StaticVirtualChannel::channel_options` ([#1517](https://github.com/Devolutions/IronRDP/issues/1517)) ([8b4c483ba0](https://github.com/Devolutions/IronRDP/commit/8b4c483ba0c900a8de0b2718347754f56dd363ba)) ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995))

- Add `StaticChannelSet::set_maximum_chunk_size` and `client_encode_svc_messages_with_max_chunk_len` to send static channel data in chunks of the negotiated `VCChunkSize`, up to `MAX_CHANNEL_CHUNK_LENGTH` ([#1622](https://github.com/Devolutions/IronRDP/issues/1622)) ([4e3903fbbe](https://github.com/Devolutions/IronRDP/commit/4e3903fbbef2904505f35e3437a7106807ac5987))

### <!-- 4 -->Bug Fixes

- Reject malformed static channel fragment sequences, such as a fragment without an initial fragment or data that does not match the declared length ([#1517](https://github.com/Devolutions/IronRDP/issues/1517)) ([8b4c483ba0](https://github.com/Devolutions/IronRDP/commit/8b4c483ba0c900a8de0b2718347754f56dd363ba))

### <!-- 7 -->Build

- Bump the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10



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
