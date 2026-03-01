# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.7.0...ironrdp-graphics-v0.7.1)] - 2026-03-01

### <!-- 1 -->Features

- Add segment wrapping utilities ([#1076](https://github.com/Devolutions/IronRDP/issues/1076)) ([5fa4964807](https://github.com/Devolutions/IronRDP/commit/5fa4964807fa15bbf1a5e3c23b365344758961aa)) 

  Adds ZGFX segment wrapping utilities for encoding data in RDP8 format.

- Add LZ77 compression support ([#1097](https://github.com/Devolutions/IronRDP/issues/1097)) ([48715483a3](https://github.com/Devolutions/IronRDP/commit/48715483a36c824af034a51f4db0580c34825d63)) 

  Adds ZGFX (RDP8) LZ77 compression to complement the existing
  decompressor, plus a high-level API for EGFX PDU preparation with
  auto/always/never mode selection.
  
  The compressor uses a hash table mapping 3-byte prefixes to history
  positions for O(1) match candidate lookup against the 2.5 MB sliding
  window.

### <!-- 4 -->Bug Fixes

- Fix pixel format handling in bitmap decoders ([#1101](https://github.com/Devolutions/IronRDP/issues/1101)) ([75863245ab](https://github.com/Devolutions/IronRDP/commit/75863245ab376f15e35c00df434860c93b123633)) 



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.6.0...ironrdp-graphics-v0.7.0)] - 2025-12-18

### Added

- [**breaking**] `InvalidIntegralConversion` variant in `RlgrError` and `ZgfxError`

### <!-- 7 -->Build

- Bump bytemuck from 1.23.2 to 1.24.0 ([#1008](https://github.com/Devolutions/IronRDP/issues/1008)) ([a24a1fa9e8](https://github.com/Devolutions/IronRDP/commit/a24a1fa9e8f1898b2fcdd41d87660ab9e38f89ed)) 

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.5.0...ironrdp-graphics-v0.6.0)] - 2025-06-27

### <!-- 4 -->Bug Fixes

- `to_64x64_ycbcr_tile` now returns a `Result`

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.4.0...ironrdp-graphics-v0.4.1)] - 2025-06-27

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 3 updates (#816) ([5c5f441bdd](https://github.com/Devolutions/IronRDP/commit/5c5f441bdd514d3fe6a29b4df872709167a9916d)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.3.0...ironrdp-graphics-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- Add helper to find diff between images ([20581bb6f1](https://github.com/Devolutions/IronRDP/commit/20581bb6f12561e22031ce0e233daeada836ea67)) 

  Add some helper to find "damaged" regions, as 64x64 tiles.

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.2.0...ironrdp-graphics-v0.3.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.2...ironrdp-graphics-v0.2.0)] - 2025-03-07

### Performance

- Replace hand-coded yuv/rgb with yuvutils ([5f1c44027a](https://github.com/Devolutions/IronRDP/commit/5f1c44027a7f6da5271565461764dd3f61729ee4)) 

  cargo bench:
  to_ycbcr                time:   [2.2988 µs 2.3251 µs 2.3517 µs]
                          change: [-83.643% -83.534% -83.421%] (p = 0.00 < 0.05)
                          Performance has improved.

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.1...ironrdp-graphics-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.0...ironrdp-graphics-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
