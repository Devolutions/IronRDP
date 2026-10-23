# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.2.1...ironrdp-core-v0.3.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Add an `offset: Option<usize>` field to the `DecodeErrorKind` and `EncodeErrorKind` variants other than `Other`, shown as `at offset N` in `Display` ([#1266](https://github.com/Devolutions/IronRDP/issues/1266)) ([a1f9189c30](https://github.com/Devolutions/IronRDP/commit/a1f9189c307516361a8faff6ecb7c1690b267998))

- [**breaking**] Add an `offset: Option<usize>` parameter to the `NotEnoughBytesErr`, `InvalidFieldErr`, `UnexpectedMessageTypeErr`, `UnsupportedVersionErr` and `UnsupportedValueErr` trait methods and to the matching `*_err` helper functions ([#1266](https://github.com/Devolutions/IronRDP/issues/1266)) ([a1f9189c30](https://github.com/Devolutions/IronRDP/commit/a1f9189c307516361a8faff6ecb7c1690b267998))

- Accept `in: <cursor>` and `at: <offset>` forms in the error macros, `cast_length!` and `cast_int!`, report the cursor position from `ensure_size!` and `ensure_fixed_part_size!`, and add an `ensure_size!(slice: ..)` form for plain slices ([#1266](https://github.com/Devolutions/IronRDP/issues/1266)) ([a1f9189c30](https://github.com/Devolutions/IronRDP/commit/a1f9189c307516361a8faff6ecb7c1690b267998)) ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec))

- Add `NonEmpty<T>`, a collection that always holds at least one element ([#1444](https://github.com/Devolutions/IronRDP/issues/1444)) ([bdcc0ceec3](https://github.com/Devolutions/IronRDP/commit/bdcc0ceec3aaa19441917db02c36cd3be2f58465))

- Add `MonotonicInstant`, a caller-supplied millisecond clock reading for sans-I/O state machines ([#1530](https://github.com/Devolutions/IronRDP/issues/1530)) ([6a499faece](https://github.com/Devolutions/IronRDP/commit/6a499faece8911e50a715a3fb08d4fd8e7d7dc87))

### <!-- 4 -->Bug Fixes

- [**breaking**] Rename the misspelled past-tense `rewind` methods of `ReadCursor` and `WriteCursor` to `rewound` ([#1529](https://github.com/Devolutions/IronRDP/issues/1529)) ([c85b089b46](https://github.com/Devolutions/IronRDP/commit/c85b089b4617176240b41482be65a77c9ad76a07))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

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
