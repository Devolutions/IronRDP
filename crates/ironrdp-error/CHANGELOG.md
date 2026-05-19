# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-error-v0.1.3...ironrdp-error-v0.2.0)] - 2026-05-19

### <!-- 1 -->Features

- Capture core::panic::Location automatically in Error<Kind> ([#1262](https://github.com/Devolutions/IronRDP/issues/1262)) ([2e2b5edfd7](https://github.com/Devolutions/IronRDP/commit/2e2b5edfd750df35bd8c8dba777ddd45c1a5bc7a)) 

  Adds automatic caller location capture to `ironrdp-error::Error<Kind>` using `#[track_caller]` + `core::panic::Location::caller()`, and surfaces that location in the `Display` output while keeping `Debug` stable for cross-platform snapshots.

- Add shared bail! and ensure! macros ([#1263](https://github.com/Devolutions/IronRDP/issues/1263)) ([68b86f2b06](https://github.com/Devolutions/IronRDP/commit/68b86f2b06ba9b09a2f9e007dd5f1783b6979cca)) 

### <!-- 4 -->Bug Fixes

- Make fields of Error private ([#1074](https://github.com/Devolutions/IronRDP/issues/1074)) ([e51ed236ce](https://github.com/Devolutions/IronRDP/commit/e51ed236ce5d55dc1a4bc5f5809fd106bdd2e834)) 

- Box diagnostic metadata to shrink Error<Kind> size ([#1269](https://github.com/Devolutions/IronRDP/issues/1269)) ([2e2699d2dc](https://github.com/Devolutions/IronRDP/commit/2e2699d2dc6644d5bbc87f41a987a2db90d281a8)) 

  Move context, location, and source into a heap-allocated ErrorMeta
  struct so that Error<Kind> holds only kind on the stack. This eliminates
  the size cascade that caused ConnectorError to exceed clippy's 128-byte
  threshold (it was raised to 152 to accommodate the Location field).
  
  Error construction is already #[cold], so the single Box allocation per
  error is acceptable.

- [**breaking**] Remove Error::into_other_kind ([#1278](https://github.com/Devolutions/IronRDP/issues/1278)) ([ac7ad50a50](https://github.com/Devolutions/IronRDP/commit/ac7ad50a501935fdf2ce0e12b6dd737dcb9aa9c9)) 

### <!-- 6 -->Documentation

- Establish the MSRV policy (current is 1.89) ([#1157](https://github.com/Devolutions/IronRDP/issues/1157)) ([c10e6ff16c](https://github.com/Devolutions/IronRDP/commit/c10e6ff16cc45f094b24e87ed1d46eb88b4a0419)) 

  The MSRV is the oldest stable Rust release that is at least 6 months
  old, bounded by the Rust version available in Debian stable-backports
  and Fedora stable.



## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-error-v0.1.1...ironrdp-error-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-error-v0.1.0...ironrdp-error-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
