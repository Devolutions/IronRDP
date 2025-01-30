# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.0...ironrdp-connector-v0.3.1)] - 2025-01-30

### <!-- 4 -->Bug Fixes

- Decrease log verbosity for license exchange (#655) ([c8597733fe](https://github.com/Devolutions/IronRDP/commit/c8597733fe9998318764064c3682506bf82026d2)) 



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.2...ironrdp-connector-v0.3.0)] - 2025-01-28

### <!-- 1 -->Features

- Support license caching (#634) ([dd221bf224](https://github.com/Devolutions/IronRDP/commit/dd221bf22401c4635798ec012724cba7e6d503b2)) 

  Adds support for license caching by storing the license obtained
  from SERVER_UPGRADE_LICENSE message and sending
  CLIENT_LICENSE_INFO if a license requested by the server is already
  stored in the cache.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

### <!-- 7 -->Build

- Bump picky from 7.0.0-rc.11 to 7.0.0-rc.12 (#639) ([a16a131e43](https://github.com/Devolutions/IronRDP/commit/a16a131e4301e0dfafe8f3b73e1a75a3a06cfdc7)) 



## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.1...ironrdp-connector-v0.2.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
