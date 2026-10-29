# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpeudp-tokio-v0.1.0...ironrdp-rdpeudp-tokio-v0.2.0)] - 2026-10-27

### <!-- 1 -->Features

- Add a STUN Binding probe in the `stun` module ([#2245](https://github.com/Devolutions/IronRDP/issues/2245)) ([79343b26cb](https://github.com/Devolutions/IronRDP/commit/79343b26cb7e57984d9ddfd33f6888adb72345c2))

- Add the optional `ice` feature, with `gather_ice_candidates`, `connect_udp_with_ice`, `accept_udp_with_ice` and `MultitransportBootstrap::connect_with_ice`, including TURN relays ([#2245](https://github.com/Devolutions/IronRDP/issues/2245)) ([79343b26cb](https://github.com/Devolutions/IronRDP/commit/79343b26cb7e57984d9ddfd33f6888adb72345c2))

- Add `connect_udp_with_socket` ([#2245](https://github.com/Devolutions/IronRDP/issues/2245)) ([79343b26cb](https://github.com/Devolutions/IronRDP/commit/79343b26cb7e57984d9ddfd33f6888adb72345c2))

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-async` public dependency to 0.12

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.11
