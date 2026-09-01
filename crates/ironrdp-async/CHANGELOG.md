# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.10.0...ironrdp-async-v0.11.0)] - 2026-09-01

### <!-- 0 -->Security

- [**breaking**] Implement multitransport bootstrapping handshake ([#1098](https://github.com/Devolutions/IronRDP/issues/1098)) ([e45fbfe0f5](https://github.com/Devolutions/IronRDP/commit/e45fbfe0f597011706e77fc174ca14e5e9d435b9)) 

  ## Summary
  
  Makes the `MultitransportBootstrapping` state functional instead of a
  no-op
  pass-through. After licensing the server may send 0, 1, or 2 Initiate
  Multitransport Request PDUs before capabilities exchange. Each one is
  surfaced
  to the application, which establishes UDP transport (RDPEUDP2 + TLS +
  RDPEMT)
  or declines, and the connector reports the outcome back to the server.
  
  ## API
  
  Mirrors the existing `should_perform_X()` pause-point pattern used by
  TLS
  upgrade and CredSSP, but uses `complete_X()` / `skip_X()` rather than
  `mark_X_as_done()` because completion carries result data:
  
  - `should_perform_multitransport()`: true while a request awaits an
  outcome
  - `multitransport_request()`: the request awaiting an outcome, or `None`
  - `complete_multitransport(result, output)`: report the outcome, resume
  - `skip_multitransport(output)`: decline, resume
  
  `complete_multitransport` accepts a `MultitransportResult` (a `Success`
  /
  `Failure(hresult)` enum) rather than a caller-built response PDU. The
  connector
  builds the response internally from the stored request ID.
  
  Requests are surfaced one at a time rather than as a batch. There is no
  end
  marker for the set, and MS-RDPBCGR 3.2.5.15.1 requires the client to act
  on a
  request as soon as it decodes one, so waiting to learn how many are
  coming is
  not an option the protocol offers. `should_perform_multitransport()` can
  therefore come round twice; the caller answers reliable and lossy
  separately.
  
  ## Approach
  
  **Routing.** Requests arrive on the negotiated MCS message channel
  (2.2.15.1) and the Demand Active on the I/O channel, so the channel
  decides
  which is which. The message channel also carries NetworkAutoDetect since
  #1348,
  so a decode still confirms what arrived there, but the I/O channel is
  never
  speculatively decoded as multitransport. A PDU on neither channel is an
  error.
  
  For the decode to be a sound confirmation the request decoder must
  reject a
  Demand Active, so this PR also tightens `MultitransportRequestPdu` to
  require
  the exact `SEC_TRANSPORT_REQ` security-header flag.
  
  **Yielding.** Each request is surfaced the moment it decodes. Responding
  returns the connector to `MultitransportBootstrapping` to read whatever
  comes
  next, which may be a second request or the Demand Active. Nothing is
  buffered
  and nothing is replayed: when the request is surfaced the Demand Active
  has not
  arrived yet.
  
  **Soft-Sync.** The Initiate Multitransport Response is the Soft-Sync
  signalling path (2.2.15.2), permitted only when both peers advertised
  `SOFTSYNC_TCP_TO_UDP` in their GCC `MultiTransportChannelData`. The
  server's
  block is retained from the GCC exchange and checked against the client's
  configured flags. One rule covers both paths:
  
  - Soft-Sync negotiated: always respond, `S_OK` or `E_ABORT`, including
  on
    `skip_multitransport()`, which 3.2.5.15.1 requires. Both the async and
  blocking drivers skip automatically, so without this every default
  client
    leaves a compliant server waiting.
  - Not negotiated: never respond. The outcome is reported in band on the
  new
  transport, and putting anything on the main channel would be the
  violation.
  
  The response goes on the message channel per 2.2.15.2 and 3.2.5.15.2. If
  Soft-Sync was negotiated but no message channel exists the connector
  errors
  rather than falling back to the I/O channel, and that check runs before
  the
  pending state is taken, so the caller is left with a connector it can
  still
  inspect or decline from.
  
  ## Wire behaviour
  
  On the wire TCP and UDP negotiation happen in parallel: the UDP
  transport is
  established alongside the ongoing TCP handshake, and its completion
  signals the
  dynamic-channel layer that subsequent channels may migrate to UDP. The
  connector's API yield point here is a Rust affordance, not a
  spec-mandated TCP
  pause. Thanks to @hardening for the correction.
  
  ## Tests
  
  Connector state-machine tests in `ironrdp-testsuite-core` drive the
  public API
  with the shared `SERVER_DEMAND_ACTIVE` fixture:
  
  - a request is surfaced on arrival, without waiting for a following PDU
    (regression test for the stall);
  - responding returns to bootstrapping so a second request is read
  normally;
  - a third request is rejected per the 2.2.15.1 cap;
  - a Demand Active on the I/O channel ends bootstrapping;
  - the response targets the message channel, decoded back off the wire;
  - a `Failure` result is carried through;
  - `skip` sends `E_ABORT` under Soft-Sync, and nothing without it;
  - `complete` emits nothing without Soft-Sync but still resumes;
  - a failed response leaves the connector in `MultitransportPending`,
  still able
    to report or decline, rather than `Consumed`;
  - `complete` / `skip` outside `MultitransportPending` error;
  - a Demand Active's user data does not decode as a
  `MultitransportRequestPdu`
    (regression test for the decoder tightening above).

### <!-- 1 -->Features

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
