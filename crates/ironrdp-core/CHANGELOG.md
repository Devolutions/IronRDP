# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.2.1...ironrdp-core-v0.3.0)] - 2026-09-04

### <!-- 1 -->Features

- Add NonEmpty<T> ([#1444](https://github.com/Devolutions/IronRDP/issues/1444)) ([bdcc0ceec3](https://github.com/Devolutions/IronRDP/commit/bdcc0ceec3aaa19441917db02c36cd3be2f58465)) 

  Add a `NonEmpty<T>` collection guaranteeing at least one element. The
  first element (head) is stored inline, so a single-element `NonEmpty`
  performs no heap allocation, and `first()` is infallible while `len()`
  returns a `NonZeroUsize`, and callers never branch on an "is it empty?"
  case.

- [**breaking**] Pass frame arrival time into Sequence::step ([#1530](https://github.com/Devolutions/IronRDP/issues/1530)) ([6a499faece](https://github.com/Devolutions/IronRDP/commit/6a499faece8911e50a715a3fb08d4fd8e7d7dc87)) 

  ## Summary
  
  - Connect-time bandwidth measurement needs to know when bytes arrived,
  and nothing in the sans-I/O layer could tell it. #1465, now merged,
  answers the server's Bandwidth Measure Stop with a nominal interval for
  exactly that reason: the connector has no way to observe the real one.
  - Introduce `MonotonicInstant`, a millisecond counter with an arbitrary
  epoch, and make `Option<MonotonicInstant>` a required parameter of
  `Sequence::step`. The I/O drivers already know when a read completed, so
  `Framed` records the arrival time of each read and hands it to the state
  machine. A driver with no clock passes `None`.
  - With arrival times available, measure for real: a Bandwidth Measure
  Start opens a window, Payload messages accumulate their byte counts, and
  Stop reports the elapsed time between its own arrival and the Start's.
  
  #1465 has merged, so this applies directly to master and carries no
  merge-order dependency. That PR was the FreeRDP unblock on its own; this
  is the design change behind it, split out at @CBenoit's suggestion in
  review.
  
  ## Why the clock lives in the driver
  
  Two reasons, both of which rule out having the sequence read a clock
  itself.

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

### <!-- 4 -->Bug Fixes

- Rename {Read,Write}Cursor::rewinded into rewound ([#1529](https://github.com/Devolutions/IronRDP/issues/1529)) ([c85b089b46](https://github.com/Devolutions/IronRDP/commit/c85b089b4617176240b41482be65a77c9ad76a07)) 



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
