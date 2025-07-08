# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.5.1...ironrdp-connector-v0.6.0)] - 2025-07-08

### <!-- 1 -->Features

- [**breaking**] Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

  Newer version of sspi adds support for server-side Kerberos.
  This is relevant for the ironrdp-acceptor crate.


# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.5.0...ironrdp-connector-v0.5.1)] - 2025-07-03

### <!-- 7 -->Build

- Bump picky to v7.0.0-rc.15 (#850) ([eca256ae10](https://github.com/Devolutions/IronRDP/commit/eca256ae10c52c4a42e7e77d41c0a1d6c180ebf3)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.4.0...ironrdp-connector-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Add no_audio_playback flag to Config struct ([9f0edcc4c9](https://github.com/Devolutions/IronRDP/commit/9f0edcc4c9c49d59cc10de37f920aae073e3dd8a)) 

  Enable audio playback on the client.

### <!-- 4 -->Bug Fixes

- [**breaking**] Fix name of client address field (#754) ([bdde2c76de](https://github.com/Devolutions/IronRDP/commit/bdde2c76ded7315f7bc91d81a0909a1cb827d870)) 

- Inject socket local address for the client addr (#759) ([712da42ded](https://github.com/Devolutions/IronRDP/commit/712da42dedc193239e457d8270d33cc70bd6a4b9)) 

  We used to inject the resolved target server address, but that is not
  what is expected. Server typically ignores this field so this was not a
  problem up until now.

### Refactor

- [**breaking**] Add supported codecs in BitmapConfig ([f03ee393a3](https://github.com/Devolutions/IronRDP/commit/f03ee393a36906114b5bcba0e88ebc6869a99785)) 



## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.2...ironrdp-connector-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.3.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.1...ironrdp-connector-v0.3.2)] - 2025-03-07

### Build

- Update dependencies



## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.0...ironrdp-connector-v0.3.1)] - 2025-01-30

### <!-- 4 -->Bug Fixes

- Decrease log verbosity for license exchange ([#655](https://github.com/Devolutions/IronRDP/issues/655)) ([c8597733fe](https://github.com/Devolutions/IronRDP/commit/c8597733fe9998318764064c3682506bf82026d2)) 



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.2...ironrdp-connector-v0.3.0)] - 2025-01-28

### <!-- 1 -->Features

- Support license caching ([#634](https://github.com/Devolutions/IronRDP/issues/634)) ([dd221bf224](https://github.com/Devolutions/IronRDP/commit/dd221bf22401c4635798ec012724cba7e6d503b2)) 

  Adds support for license caching by storing the license obtained
  from SERVER_UPGRADE_LICENSE message and sending
  CLIENT_LICENSE_INFO if a license requested by the server is already
  stored in the cache.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo ([#631](https://github.com/Devolutions/IronRDP/issues/631)) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

### <!-- 7 -->Build

- Bump picky from 7.0.0-rc.11 to 7.0.0-rc.12 ([#639](https://github.com/Devolutions/IronRDP/issues/639)) ([a16a131e43](https://github.com/Devolutions/IronRDP/commit/a16a131e4301e0dfafe8f3b73e1a75a3a06cfdc7)) 



## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.1...ironrdp-connector-v0.2.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
