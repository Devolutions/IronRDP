# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.4.0...ironrdp-rdpsnd-native-v0.4.1)] - 2025-09-24

### <!-- 7 -->Build

- Replace `opus` by `opus2` (#985) ([e5042a7d81](https://github.com/Devolutions/IronRDP/commit/e5042a7d81b864e78ccf19d6b358d94458f951d0)) 

  `opus` is unmaintained and points to a 4-year-old commit of the opus C
  library. This does not compile anymore on our CI, because their
  CMakeList.txt requires an older version of CMake that is not available
  in the runners we use. `opus2` is a fork that points to a more recent
  version of it.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.3.1...ironrdp-rdpsnd-native-v0.4.0)] - 2025-08-29

### <!-- 7 -->Build

- Bump cpal to 0.16 ([eeac1fee1f](https://github.com/Devolutions/IronRDP/commit/eeac1fee1fed4858f4776d86072790bc074e34eb)) 

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.3.0...ironrdp-rdpsnd-native-v0.3.1)] - 2025-06-27

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 3 updates (#816) ([5c5f441bdd](https://github.com/Devolutions/IronRDP/commit/5c5f441bdd514d3fe6a29b4df872709167a9916d)) 

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.3...ironrdp-rdpsnd-native-v0.1.4)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.2...ironrdp-rdpsnd-native-v0.1.3)] - 2025-02-05

### <!-- 1 -->Features

- Add Opus audio client decoding (#661) ([ccf6348270](https://github.com/Devolutions/IronRDP/commit/ccf63482706ecfbbdc6038028ea2ee086d0e3640)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.1...ironrdp-rdpsnd-native-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.0...ironrdp-rdpsnd-native-v0.1.1)] - 2024-12-15

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
