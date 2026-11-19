# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.0.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpel-v0.0.2...ironrdp-rdpel-v0.0.3)] - 2026-11-19

### <!-- 1 -->Features

- Add MS-RDPEL location channel processor ([#2442](https://github.com/Devolutions/IronRDP/issues/2442)) ([843d02fa0e](https://github.com/Devolutions/IronRDP/commit/843d02fa0e575dd395e8dea351a0c3ec6106ca83)) 

  Implement `LocationServer`, `LocationHandler`, and `LocationFix` to
  decode client `[MS-RDPEL]` location updates over the
  `Microsoft::Windows::RDS::Location` dynamic virtual channel. Reconstruct
  `BaseLocation3d`, `Location2dDelta`, and `Location3dDelta` PDUs in exact
  integer nanodegrees, bound incoming DVC messages with
  `max_message_size`, and invalidate the delta base when an update is
  malformed or out of range per `[MS-RDPEL]` section 3.3.5.4.



## [[0.0.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpel-v0.0.1...ironrdp-rdpel-v0.0.2)] - 2026-10-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.11

- [**breaking**] Update `ironrdp-svc` public dependency to 0.10



## [[0.0.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpel-v0.0.0...ironrdp-rdpel-v0.0.1)] - 2026-10-23

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.9

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9
