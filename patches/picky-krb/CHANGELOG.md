# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.12.0](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.11.3...picky-krb-v0.12.0)] - 2025-10-20

### <!-- 7 -->Fix

- [**breaking**] Fix typo in field name of `EncKdcRepPart` ([#434](https://github.com/Devolutions/picky-rs/pull/434))

## [[0.11.3](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.11.2...picky-krb-v0.11.3)] - 2025-10-10

### <!-- 7 -->Build

- Pin RustCrypto release candidate crates ([#417](https://github.com/Devolutions/picky-rs/issues/417)) ([8a79282bbc](https://github.com/Devolutions/picky-rs/commit/8a79282bbc0dae9df222f16d261b7dd1f03cd66f)) 

## [[0.11.2](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.11.1...picky-krb-v0.11.2)] - 2025-09-26

### <!-- 7 -->Build

- Bump the crypto group across 1 directory with 3 updates (#388) ([58d179a0c3](https://github.com/Devolutions/picky-rs/commit/58d179a0c39d701025a363c3f294912c2881a8f5)) 

### Changed

- Bump minimal rustc version to 1.85.

## [[0.11.1](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.11.0...picky-krb-v0.11.1)] - 2025-08-18

### <!-- 7 -->Build

- Bump uuid from 1.17.0 to 1.18.0 (#393) ([c4c6280a2c](https://github.com/Devolutions/picky-rs/commit/c4c6280a2c51bef81a509edd06a96c165345f88d)) 

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.11.0](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.10.0...picky-krb-v0.11.0)] - 2025-06-24

### <!-- 4 -->Bug Fixes

- `EncTicketPart` structure; (#381) ([79ed323732](https://github.com/Devolutions/picky-rs/commit/79ed323732efbbef18030011ec1926239d8f6175)) 

## [[0.10.0](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.9.6...picky-krb-v0.10.0)] - 2025-06-04

### <!-- 4 -->Bug Fixes

- **[breaking]** Fix typo in field name `AuthenticatorInner::authenticator_vno` (#373) ([b3ae4ab263](https://github.com/Devolutions/picky-rs/commit/b3ae4ab263234925b42e91d47ae36d52eeae1693)) 

## [[0.9.6](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.9.5...picky-krb-v0.9.6)] - 2025-05-26

### <!-- 1 -->Features

- Deserialize for `KrbMessage/ApplicationTag0` (#369) ([6eac340241](https://github.com/Devolutions/picky-rs/commit/6eac3402416981409bf1d211bed1ff3b99eaebcf)) 

### <!-- 7 -->Build

- Bump uuid from 1.16.0 to 1.17.0 (#370) ([af8ac1be76](https://github.com/Devolutions/picky-rs/commit/af8ac1be7654fd5b54deb80bb816c7865883bc41)) 

## [[0.9.5](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.9.4...picky-krb-v0.9.5)] - 2025-05-14

### <!-- 1 -->Features

- Implement `EncTicketPart` (#366) ([bcf71a1688](https://github.com/Devolutions/picky-rs/commit/bcf71a1688a74d2b9c6475c987b48b80b077d361)) 


# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.4](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.9.3...picky-krb-v0.9.4)] - 2025-02-21

### <!-- 1 -->Features

- `encryption_checksum` for krb ciphers ([#344](https://github.com/Devolutions/picky-rs/pull/344)) ([a75f0cdc50](https://github.com/Devolutions/picky-rs/commit/a75f0cdc5003d12ba0e6e64fe2385de34da1b32f)) 

## [[0.9.3](https://github.com/Devolutions/picky-rs/compare/picky-krb-v0.9.2...picky-krb-v0.9.3)] - 2025-02-04

### <!-- 0 -->Security

- Implement Kerberos encryption without a checksum ([#342](https://github.com/Devolutions/picky-rs/pull/342)) ([90eab0150a](https://github.com/Devolutions/picky-rs/commit/90eab0150a6645b667ad2eb49085f0de5556ebd2)) 

  Added the possibility of Kerberos encryption but without a checksum.
  This functionality is needed to support `SECBUFFER_READONLY` and
  `SECBUFFER_READONLY_WITH_CHECKSUM` flags for security buffers in `sspi-rs`.

### <!-- 4 -->Bug Fixes

- Symlinks to license files in packages ([#339](https://github.com/Devolutions/picky-rs/pull/339)) ([1834c04f39](https://github.com/Devolutions/picky-rs/commit/1834c04f3930fb1bbf040deb6525b166e378b8aa)) 

  Use symlinks instead of copying files to avoid a “dirty” state during
  cargo publish and preserve VCS info. With #337 merged, CI handles
  publishing consistently, so developer environments no longer matter.


## [0.9.2] 2024-11-26

### Changed

- Update dependencies

## [0.9.1] 2024-11-19

### Changed

- Update dependencies

## [0.9.0] 2024-07-12

### Changed

- Bump minimal rustc version to 1.61
- Update dependencies

## [0.8.0] 2023-08-24

### Fixed

- License files are now correctly included in the published package
- Creds and key spec constants
- Credssp password and smartcard structs

### Changed

- Update dependencies

## [0.7.1]

### Changed

- Update dependencies

## [0.7.0]

### Improvement

- Pretty string representation and description for error codes

## [0.6.0] 2023-02-14

### Added

- Add Kerberos error codes([#199](https://github.com/Devolutions/picky-rs/pull/199))
- Fix ToString impl for KrbErrorInner ([#194](https://github.com/Devolutions/picky-rs/pull/194))

## [0.5.0] 2022-11-07

### Added

- Useful features for PKU2U support in sspi-rs ([#186](https://github.com/Devolutions/picky-rs/pull/186))

## [0.4.0] 2022-09-01

### Added

-  Kerberos crypto algorithms([#173](https://github.com/Devolutions/picky-rs/pull/173))

## [0.3.1] 2022-07-28

### Added

- Add constants related to SECBUFFER_CHANNEL_BINDINGS([#163](https://github.com/Devolutions/picky-rs/pull/163))

## [0.3.0] 2022-07-18

### Added

- Kerberos "Change password" protocol ([#155](https://github.com/Devolutions/picky-rs/pull/155))

## [0.2.0] 2022-05-27

### Added

- Missing Kerberos name type constants ([#150](https://github.com/Devolutions/picky-rs/pull/150))

## [0.1.0] 2022-05-19

Initial version

