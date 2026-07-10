# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.17.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.16.0...ironrdp-v0.17.0)] - 2026-07-10

### <!-- 0 -->Security

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 1 -->Features

- Gate native backends behind Cargo features ([#1338](https://github.com/Devolutions/IronRDP/issues/1338)) ([f7e6106e0f](https://github.com/Devolutions/IronRDP/commit/f7e6106e0f293c1e0f8129be82aa2d86737ba92a)) 

  - Added:    client, client-all, client-sound, client-clipboard,
              client-rdpdr, client-smartcard, client-gateway,
              client-dvc-pipe-proxy, client-dvc-com-plugin, and
              top-level rustls / native-tls (forwarded to ironrdp-client)
  - Modified: qoi, qoiz now also gate ironrdp-client's codec

- [**breaking**] Misuse-resistant format negotiation for RdpsndServerHandler ([#1359](https://github.com/Devolutions/IronRDP/issues/1359)) ([2d3bdef1a7](https://github.com/Devolutions/IronRDP/commit/2d3bdef1a7167d2acdc478a92917cbb2f018960b)) 

  Move the negotiation into the crate and split selection from lifecycle:
  
  ```rust
  fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat>;
  fn start(&mut self, format: &NegotiatedFormat);
  ```

### <!-- 4 -->Bug Fixes

- [**breaking**] Remove ironrdp-connector dependency ([#1435](https://github.com/Devolutions/IronRDP/issues/1435)) ([c6a0286dcb](https://github.com/Devolutions/IronRDP/commit/c6a0286dcb49d9ac54c65c4f9325b41e05d541b8)) 

  Removes the last ironrdp-connector coupling from ironrdp-session by
  turning Deactivate-All handling into a bare signal and shifting ownership
  of the Deactivation-Reactivation activation sequence back to each consumer.
  It introduces a ConnectionActivationFactory (fresh sequence per reactivation)
  and an ActiveStageBuilder so session construction no longer depends on
  ConnectionResult.



## [[0.16.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.15.0...ironrdp-v0.16.0)] - 2026-06-05

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-displaycontrol`, `ironrdp-dvc`, `ironrdp-echo`, `ironrdp-server`, and `ironrdp-session` public dependencies



## [[0.15.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.14.0...ironrdp-v0.15.0)] - 2026-05-27

### Build

- Update dependencies

## [[0.14.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.13.0...ironrdp-v0.14.0)] - 2025-12-18

### <!-- 7 -->Build

- Bump picky and sspi ([#1028](https://github.com/Devolutions/IronRDP/issues/1028)) ([5bd319126d](https://github.com/Devolutions/IronRDP/commit/5bd319126d32fbd8e505508e27ab2b1a18a83d04)) 

  This fixes build issues with some dependencies.

## [[0.13.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.12.0...ironrdp-v0.13.0)] - 2025-09-24

### Build

- Update dependencies

## [[0.12.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.11.0...ironrdp-v0.12.0)] - 2025-08-29

### Build

- Update dependencies

## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.10.0...ironrdp-v0.11.0)] - 2025-07-08

### Build

- Update dependencies

## [[0.9.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.9.0...ironrdp-v0.9.1)] - 2025-03-13

### <!-- 6 -->Documentation

- Fix documentation build (#700) ([0705840aa5](https://github.com/Devolutions/IronRDP/commit/0705840aa51bc920e76f0cf1fce06b29733c6e2d)) 

## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.8.0...ironrdp-v0.9.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.4...ironrdp-v0.8.0)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.7.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.3...ironrdp-v0.7.4)] - 2025-01-28

### Build

- Update dependencies

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

- Extend server example to demonstrate Opus audio codec support (#643) ([fa353765af](https://github.com/Devolutions/IronRDP/commit/fa353765af016734c07e31fff44d19dabfdd4199)) 


## [[0.7.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.2...ironrdp-v0.7.3)] - 2024-12-16

### <!-- 6 -->Documentation

- Inline documentation for re-exported items (#619) ([cff5c1a59c](https://github.com/Devolutions/IronRDP/commit/cff5c1a59cdc2da73cabcb675fcf2d85dc81fd68)) 


## [[0.7.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.1...ironrdp-v0.7.2)] - 2024-12-15

### <!-- 6 -->Documentation

- Fix server example ([#616](https://github.com/Devolutions/IronRDP/pull/616)) ([02c6fd5dfe](https://github.com/Devolutions/IronRDP/commit/02c6fd5dfe142b7cc6f15cb17292504657818498)) 

  The rt-multi-thread feature of tokio is not enabled when compiling the
  example alone (without feature unification from other crates of the
  workspace).


## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.0...ironrdp-v0.7.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
