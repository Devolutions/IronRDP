# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.8.0...ironrdp-displaycontrol-v0.9.0)] - 2026-08-29

### <!-- 1 -->Features

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

- Decode the full headered DISPLAYCONTROL_CAPS_PDU ([#1442](https://github.com/Devolutions/IronRDP/issues/1442)) ([3b66961a8b](https://github.com/Devolutions/IronRDP/commit/3b66961a8b2ec5bb2d49175c6970e4a480348b3f)) 

  ## Summary



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.7.0...ironrdp-displaycontrol-v0.8.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.8

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.6.0...ironrdp-displaycontrol-v0.7.0)] - 2026-06-05

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.5.0...ironrdp-displaycontrol-v0.6.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-core`, `ironrdp-dvc`, `ironrdp-pdu`, and `ironrdp-svc` public dependencies

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.3...ironrdp-displaycontrol-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.2...ironrdp-displaycontrol-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.1...ironrdp-displaycontrol-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.0...ironrdp-displaycontrol-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
