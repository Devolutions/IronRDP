# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpeudp-tokio-v0.1.0...ironrdp-rdpeudp-tokio-v0.2.0)] - 2026-10-27

### <!-- 1 -->Features

- Support STUN and ICE candidate gathering ([#2245](https://github.com/Devolutions/IronRDP/issues/2245)) ([79343b26cb](https://github.com/Devolutions/IronRDP/commit/79343b26cb7e57984d9ddfd33f6888adb72345c2)) 

  Add a standalone RFC 8489 STUN Binding probe and an optional `ice`
  feature to `ironrdp-rdpeudp-tokio` for UDP ICE candidate gathering,
  connectivity checks, nominated-pair selection, and UDP TURN relay
  transport beneath the existing RDPEUDP2, TLS, and RDPEMT stack.
  
  Expose `connect_udp_with_socket`, `gather_ice_candidates`,
  `connect_udp_with_ice`, `accept_udp_with_ice`, and
  `MultitransportBootstrap::connect_with_ice` while keeping the default
  direct `connect_udp` path unchanged.


