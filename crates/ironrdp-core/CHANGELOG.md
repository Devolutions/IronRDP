# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.2.0...ironrdp-core-v0.2.1)] - 2026-07-10

### <!-- 1 -->Features

- Add `WriteBuf::filled_mut`, the mutable counterpart of `filled` ([#1374](https://github.com/Devolutions/IronRDP/issues/1374)) ([d3705af18c](https://github.com/Devolutions/IronRDP/commit/d3705af18cff1851f4d48017affcb85aaa678d57)) 

### <!-- 4 -->Bug Fixes

- Propagate caller location through error constructor helpers ([#1392](https://github.com/Devolutions/IronRDP/issues/1392)) ([d6990d81a1](https://github.com/Devolutions/IronRDP/commit/d6990d81a17e8349e52768ad8a82f673b1e1462d)) 

  The error constructor helpers in several crates wrap the #[track_caller]
  ironrdp_error::Error::new, but were not themselves marked
  #[track_caller]. As a result, the captured location pointed at the
  helper body instead of the real call site, giving misleading "@
  file:line" info in error reports.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.5...ironrdp-core-v0.2.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-error` public dependency to 0.2

## [[0.1.5](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.4...ironrdp-core-v0.1.5)] - 2025-05-28

### Features

- Adds `write_padding` and `read_padding` functions/macros extracted from `ironrdp-pdu` crate

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.3...ironrdp-core-v0.1.4)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.2...ironrdp-core-v0.1.3)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.1...ironrdp-core-v0.1.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
