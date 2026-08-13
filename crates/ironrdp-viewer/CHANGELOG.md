# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-viewer-v0.1.0...ironrdp-viewer-v0.1.1)] - 2026-08-13

### <!-- 1 -->Features

- Expose generic session configuration and lifecycle APIs ([#1522](https://github.com/Devolutions/IronRDP/issues/1522)) ([57b1366650](https://github.com/Devolutions/IronRDP/commit/57b13666506dc40c15b4c4702d35150beee99133)) 

  ## Summary
  - expose generic client configuration for connection metadata,
  compression, shell/work directory, audio, and runtime static-channel
  factories
  - add bounded input delivery with independent close cancellation, host
  clipboard plumbing, lifecycle events, and Display Control resize
  readiness/fallback handling
  - update agent, viewer, web, FFI, examples, and tests for the generic
  APIs
  
  ## Stack dependencies
  This PR is stacked on `copilot/tls-validation-policy` (`b2bbcece`),
  which already includes the merged runtime static-channel support from
  `master`. It intentionally contains no TLS implementation/policy,
  ActiveX/COM, SVC implementation, decompression, or bitmap-recovery
  changes.
  
  ## Validation
  - `cargo fmt --check --all`
  - `cargo xtask check tests --no-run -v`
  - `cargo xtask check lints -v`
  - `cargo test -p ironrdp-client --lib --features rustls`
  - `cargo check -p ironrdp-agent -p ironrdp-viewer -p ironrdp-web -p ffi`
  
  ---------

- Add IronRDP ActiveX COM server ([#1523](https://github.com/Devolutions/IronRDP/issues/1523)) ([ee58b7c5f2](https://github.com/Devolutions/IronRDP/commit/ee58b7c5f283ef64be93a8483242f49070c6cdc9)) 

  ## Summary
  - Add the IronRDP ActiveX COM server and native MSTSC host integration.
  - Provide bounded native-host diagnostics, credential-bridge support,
  and an AxHost test harness.
  - Preserve the minimal client, connector, and error integrations needed
  by the control.
  
  ## Validation
  - cargo check -p ironrdp-activex
  - Focused ironrdp-error and ironrdp-connector tests
  - cargo test -p ironrdp-activex --lib --no-run
  - cargo fmt --all -- --check
  - cargo xtask check locks -v
  
  ---------

- Host agent RPC endpoint ([#1545](https://github.com/Devolutions/IronRDP/issues/1545)) ([358ca8d4f3](https://github.com/Devolutions/IronRDP/commit/358ca8d4f382e49251f91243b6d9e5488c2dface)) 

  ## Summary
  - host the existing `ironrdp-agent` RPC contract from `ironrdp-viewer
  --rpc`
  - retain one shared daemon state for GUI and RPC input, frames, session
  lifecycle, screenshots, logs, and NOW operations
  - use the agent's default endpoint so `ironrdp-agent` itself remains
  unchanged
  
  ## Usage
  Start `ironrdp-viewer --rpc` before invoking `ironrdp-agent`; the viewer
  owns the usual agent endpoint until its window closes. Use matching
  explicit `--rpc-endpoint` and agent `--endpoint` values for a custom
  endpoint.
  
  ## Validation
  - `cargo fmt --check`
  - `cargo test -p ironrdp-viewer`
  - `cargo test -p ironrdp-daemon -p ironrdp-agent --lib`
  - `cargo check -p ironrdp-viewer -p ironrdp-daemon -p ironrdp-agent`

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

- Add RemoteApp channel support ([#1637](https://github.com/Devolutions/IronRDP/issues/1637)) ([ab48c6cb8c](https://github.com/Devolutions/IronRDP/commit/ab48c6cb8c017504f8a92799aeb91b821c50a13a)) 

  Configure and negotiate RAIL connections, then route its static channel
  through the portable client with bounded request queues and server
  control events.

- Project RemoteApp windows ([#1641](https://github.com/Devolutions/IronRDP/issues/1641)) ([f5554f40dc](https://github.com/Devolutions/IronRDP/commit/f5554f40dc280d93506ea8352e3992e641d58e96)) 

  Project server-authoritative RAIL windows for an enabled ActiveX
  RemoteApp session and launch the configured program after RAIL becomes
  available.
  
  Forward validated opaque windowing orders to the worker, maintain their
  basic HWND lifecycle, and retain windows until the server removes them.
  Leave desktop behavior and unsupported shell features unchanged.

- Add RAIL audit commands ([#1646](https://github.com/Devolutions/IronRDP/issues/1646)) ([77759dca03](https://github.com/Devolutions/IronRDP/commit/77759dca032eb829b5be54a3c44d9be92252cf41)) 

  Expose bounded client-validated RAIL events and RemoteApp launch
  requests through the daemon IPC so headless agents can verify sessions.
  
  Preserve cursors across resize reconnects, wake waiting readers for
  locally queued launches, and redact launch data in logs.
  
  Report terminal local Execute failures without disrupting an otherwise
  valid RDP session.

### <!-- 4 -->Bug Fixes

- Make source locations opt-in ([#1480](https://github.com/Devolutions/IronRDP/issues/1480)) ([f84cd01450](https://github.com/Devolutions/IronRDP/commit/f84cd01450e18d12838b225859878b311802b805)) 

  Default error display omits locations; alternate formatting and reports
  with explicit location opt-in preserve diagnostic context.



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-viewer-v0.1.0)] - 2026-07-10

Initial release.
