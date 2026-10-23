# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.10.0...ironrdp-async-v0.11.0)] - 2026-10-23

### <!-- 0 -->Security

- [**breaking**] Follow server redirection to the target host ([#2208](https://github.com/Devolutions/IronRDP/issues/2208)) ([485657a6ec](https://github.com/Devolutions/IronRDP/commit/485657a6eca682a0356e100a9fda229342861fd7)) 

  An RD Connection Broker sends a Server Redirection PDU to move a client
  to the session host that holds or will hold its session. IronRDP
  rejected the PDU as an unexpected share control PDU, so a client could
  not connect to an RDS farm through its broker.
  
  The PDU layer decodes and encodes the Server Redirection Packet with
  every optional field, the target net addresses and the Base64 target
  certificate container, in both its Enhanced Security variant (share
  control type PDUTYPE_SERVER_REDIR_PKT) and its Standard Security
  variant. The Client Info PDU can carry an opaque password cookie
  verbatim, as FreeRDP sends it.
  
  The connector recognizes the PDU during capabilities exchange and
  connection finalization and ends in a `Redirected` state instead of
  failing; `connect_finalize` returns a `ConnectionOutcome` that is either
  `Connected` or `Redirected`. The session reports a redirection received
  later as `ActiveStageOutput::ServerRedirection`. PDUs marked
  LB_NOREDIRECT are ignored.
  
  `ironrdp-client` now always advertises redirection support in the Client
  Cluster Data and follows a redirection for every front-end: it closes
  the connection, emits `RdpOutputEvent::ServerRedirected` and reconnects
  with the load-balancing information as the X.224 routing token, the
  session ID in the cluster data, the password cookie in the Client Info
  PDU and the redirected user name and domain. Transport, TLS and CredSSP
  settings are kept. A direct connection tries the target addresses first,
  like mstsc, and authenticates the FQDN; `use redirection server name`
  (ActiveX `UseRedirectionServerName`) or a gateway puts the names first.
  A target certificate reported by the broker is accepted under strict
  validation, a TsvUrl mismatch aborts, and at most 8 redirections are
  followed. RDSTLS-encrypted passwords are refused as unsupported.
  
  `ironrdp-server` gains a redirection handle that sends the PDU to the
  active client and waits for it to disconnect, and reports the routing
  token, redirected session ID and password cookie of an incoming
  connection. The FFI exposes the redirection outcome to .NET.

### <!-- 1 -->Features

- [**breaking**] Tolerate unrecognized device-reported USB capability values ([#1418](https://github.com/Devolutions/IronRDP/issues/1418)) ([b2d75d4862](https://github.com/Devolutions/IronRDP/commit/b2d75d4862b4549b0a63cf9b26f02c3e8813014b)) 

  ## Problem

- Hyper-V vmconnect support ([#1503](https://github.com/Devolutions/IronRDP/issues/1503)) ([a7cc067d50](https://github.com/Devolutions/IronRDP/commit/a7cc067d5069cbbcb13bae3e0561c0611da3bcf6)) 

  Adds Hyper-V VMConnect's direct ordering: PCB → TLS → CredSSP → X.224.
  
  Enhanced Session is the default (`GUID;EnhancedMode=1`), with
  `--vmconnect-basic` for the synthetic console. Kept this separate in
  `ironrdp-vmconnect`; no SPN changes.
  
  Tested against the nested Hyper-V lab:
  - Enhanced: `HYBRID_EX`, rendered 1280×720
  - Basic: `HYBRID`, rendered 1280×720
  - `cargo xtask check fmt/lints/tests -v`
  
  ---------

- Support Hyper-V connection ordering ([#1505](https://github.com/Devolutions/IronRDP/issues/1505)) ([5c1816244e](https://github.com/Devolutions/IronRDP/commit/5c1816244e83187a04249e9d9c240d096cb78f55)) 

  Hyper-V over RDCleanPath needs PCB → TLS on the proxy, then CredSSP →
  X.224 on the client. Ordinary RDCleanPath stays X.224-first.
  
  Still VERSION_1 with the same DER fields. An explicit VMConnect request
  carries a Unicode PCB payload in `preconnection_blob` with no X.224; the
  proxy encodes the binary PCB. Generic PCB requests keep their existing
  X.224-first behavior.
  
  Gateway reference implementation:
  [Devolutions/devolutions-gateway#1372](https://github.com/Devolutions/devolutions-gateway/pull/1372)
  
  Checked locally: Rust builds, formatting, Svelte typecheck, and .NET
  build. Real nested Hyper-V E2E through Gateway: Native rendered 18
  frames, Avalonia connected and rendered its first frame, and Web
  rendered a non-empty 1280×720 canvas.
  
  ---------

- [**breaking**] Pass frame arrival time into Sequence::step ([#1530](https://github.com/Devolutions/IronRDP/issues/1530)) ([6a499faece](https://github.com/Devolutions/IronRDP/commit/6a499faece8911e50a715a3fb08d4fd8e7d7dc87)) 

  ## Summary
  
  - Connect-time bandwidth measurement needs to know when bytes arrived,
  and nothing in the sans-I/O layer could tell it. #1465, now merged,
  answers the server's Bandwidth Measure Stop with a nominal interval for
  exactly that reason: the connector has no way to observe the real one.
  - Introduce `MonotonicInstant`, a millisecond counter with an arbitrary
  epoch, and make `Option<MonotonicInstant>` a required parameter of
  `Sequence::step`. The I/O drivers already know when a read completed, so
  `Framed` records the arrival time of each read and hands it to the state
  machine. A driver with no clock passes `None`.
  - With arrival times available, measure for real: a Bandwidth Measure
  Start opens a window, Payload messages accumulate their byte counts, and
  Stop reports the elapsed time between its own arrival and the Start's.
  
  #1465 has merged, so this applies directly to master and carries no
  merge-order dependency. That PR was the FreeRDP unblock on its own; this
  is the design change behind it, split out at @CBenoit's suggestion in
  review.
  
  ## Why the clock lives in the driver
  
  Two reasons, both of which rule out having the sequence read a clock
  itself.

- Delegate multitransport setup ([#1858](https://github.com/Devolutions/IronRDP/issues/1858)) ([7036fb8c7e](https://github.com/Devolutions/IronRDP/commit/7036fb8c7ef32f71b456745902e14d34008c7add)) 

  Let applications establish negotiated multitransport channels while the
  async connector retains protocol sequencing and response ownership.
  
  Expose Soft-Sync state to the setup callback, centralize response
  construction, and report callback failures with E_ABORT when possible.
  The existing finalizer still declines multitransport and uses TCP.

### <!-- 4 -->Bug Fixes

- Reject a zero-length unmatched PDU in read_by_hint ([#1556](https://github.com/Devolutions/IronRDP/issues/1556)) ([2d2c37fc21](https://github.com/Devolutions/IronRDP/commit/2d2c37fc21bd7f089fbbca41831abba14fa2b72b)) 

  ## Problem



## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.9.0...ironrdp-async-v0.10.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-connector` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.8.0...ironrdp-async-v0.9.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- [**breaking**] Make Framed::read_exact crate-private ([#1247](https://github.com/Devolutions/IronRDP/issues/1247)) ([d02d24aad4](https://github.com/Devolutions/IronRDP/commit/d02d24aad44039c0425a022f1bd9677800706cea)) 


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.7.0...ironrdp-async-v0.8.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

## [[0.3.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.3.1...ironrdp-async-v0.3.2)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.3.0...ironrdp-async-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.2.1...ironrdp-async-v0.3.0)] - 2025-01-28

### <!-- 4 -->Changed

- Remove unmatched parameter from `Framed::read_by_hint` function ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.2.0...ironrdp-async-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
