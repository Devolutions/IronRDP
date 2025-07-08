# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.5.1...ironrdp-tokio-v0.6.0)] - 2025-07-08

### Build

- Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.5.0...ironrdp-tokio-v0.5.1)] - 2025-07-08

### <!-- 1 -->Features

- Add async `ReqwestNetworkClient::send` method (#859) ([7e23a8bb97](https://github.com/Devolutions/IronRDP/commit/7e23a8bb97991d0e24e65d77a11d9854492ee024)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.4.0...ironrdp-tokio-v0.5.0)] - 2025-06-06

### <!-- 4 -->Bug Fixes

- [**breaking**] Adjust reqwest-related features (#812) ([9408789491](https://github.com/Devolutions/IronRDP/commit/9408789491b3e09b69e0aaa03fd215326b624ec0)) 

  - Remove `reqwest` from the default feature set.
  - Disable default TLS backend.
  - Add `reqwest-rustls-ring` to enable rustls + ring backend.
  - Add `reqwest-native-tls` to enable native-tls backend.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.3.0...ironrdp-tokio-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- Add reqwest feature (#734) ([032c38be92](https://github.com/Devolutions/IronRDP/commit/032c38be9229cfd35f0f6fc8eac5cccc960480d3)) 

## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.2...ironrdp-tokio-v0.2.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.1...ironrdp-tokio-v0.2.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.0...ironrdp-tokio-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
