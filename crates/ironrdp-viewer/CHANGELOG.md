# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-viewer-v0.1.0...ironrdp-viewer-v0.2.0)] - 2026-08-22

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

- Add native MS-RDPEWA WebAuthn redirection ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb)) 

  Implement the RDPEWA dynamic channel with a Windows WebAuthn backend and
  wire RedirectWebAuthn for ActiveX, the optional client feature, and the
  viewer CLI.
  
  Prefer System32\webauthn.dll via the DVC COM plugin for MSTSC parity.
  The pure-Rust backend forwards ceremonies through a webauthn.dll IWTS
  oneshot so hash-only hosts that omit clientDataJSON still work; public
  WebAuthN* remains a fallback when JSON is present. Recreate
  WebAuthN_Channel opens through shared COM/listener factories because
  Windows opens and closes the channel around each RPC.
  
  Side effects:
  - New crates ironrdp-rdpewa and ironrdp-rdpewa-native
  - Config key redirectwebauthn; ActiveX ExtendedSettings property
  - ironrdp-daemon webauthn feature for ironrdp-agent
  - IRONRDP_WEBAUTHN_FORCE_NATIVE debug switch
  - Viewer --webauthn/--no-webauthn flags; .rdp redirectwebauthn default
  - ActiveX docs note no AdvancedSettings slot and no IPersist persistence

- Support automatic reconnect ([#1662](https://github.com/Devolutions/IronRDP/issues/1662)) ([34d7b31dc5](https://github.com/Devolutions/IronRDP/commit/34d7b31dc5039273dc5502d0ca3301f256eb6154)) 

  Support automatic reconnect through the ActiveX control with bounded
  host-approved retries after transport loss.
  
  Reuse ARC cookies only for resumed sessions, reject server ARC status
  failures, and report success after active-session traffic.
  
  ---------

- Enable Windows smartcard in product surfaces ([#1672](https://github.com/Devolutions/IronRDP/issues/1672)) ([1561a4c327](https://github.com/Devolutions/IronRDP/commit/1561a4c327512a1a3a6c618d4b06dc388fab132f)) 

  Wire daemon, agent, viewer, and ActiveX to the WinSCard RDPDR backend so
  products can announce a smartcard device only with a matching factory.
  
  Expose `WindowsRdpdrBackendFactory::with_smartcard`, daemon/agent
  `--smartcard` and `ironrdp_smartcard` connect overrides, viewer
  `--smartcard`, and ActiveX `RedirectSmartCards`. Smartcard-only RDPDR is
  valid on Windows; non-Windows paths refuse or hard-disable enablement.

- Negotiate monitor topology ([#1675](https://github.com/Devolutions/IronRDP/issues/1675)) ([063efcdc30](https://github.com/Devolutions/IronRDP/commit/063efcdc3088d8f44e423cc322077d40bf9aadf2)) 

  Negotiate the client monitor layout from UseMultimon and expose the
  confirmed remote topology through the ActiveX compatibility interface.
  
  Advertise Monitor Layout PDU support whenever Extended Client Data is
  negotiated, and forward layouts from activation, active sessions, and
  reactivation so advertised support does not terminate sessions.
  
  Keep fallback reporting truthful when servers do not honor the request,
  while preserving single-monitor resize behavior and blocking
  multi-monitor resizing.
  
  Do not send Client Monitor Extended Data; per-monitor DPI and
  orientation remain unavailable.

- Try a direct connection before the RD Gateway ([#1713](https://github.com/Devolutions/IronRDP/issues/1713)) ([a67dbf47e4](https://github.com/Devolutions/IronRDP/commit/a67dbf47e429a73b29e2b36966b6dc5b870625cd)) 

### <!-- 4 -->Bug Fixes

- Make source locations opt-in ([#1480](https://github.com/Devolutions/IronRDP/issues/1480)) ([f84cd01450](https://github.com/Devolutions/IronRDP/commit/f84cd01450e18d12838b225859878b311802b805)) 

  Default error display omits locations; alternate formatting and reports
  with explicit location opt-in preserve diagnostic context.

- [**breaking**] Accept the full documented range of Client Core Data keyboardType values ([#1689](https://github.com/Devolutions/IronRDP/issues/1689)) ([c74e7c5c94](https://github.com/Devolutions/IronRDP/commit/c74e7c5c9431c87e57a1550b037867d235c1d362)) 

  ## Summary
  
  KeyboardType (TS_UD_CS_CORE's keyboardType field, MS-RDPBCGR 2.2.1.3.2)
  was a closed enum covering discriminants 1 through 7, transcribed from
  the field's other documentation in 2.2.7.1.6 (TS_INPUT_CAPABILITYSET),
  which omits value 8 (Korean keyboard). The table this type actually
  decodes against, 2.2.1.3.2, documents 1 through 8. Any client outside
  the narrower range, including every genuine Korean keyboard, had its
  whole Client Core Data parse hard rejected before the connection
  started.
  
  Converted KeyboardType to a repr(transparent) struct KeyboardType(pub
  u32) with named constants for the eight documented values, mirroring
  RdpVersion one field above it in the same struct, and made decode
  infallible at all three sites that previously disagreed with each other:
  gcc::core_data::client hard rejected, rdp::capability_sets::input
  silently dropped to None, and ironrdp-activex's COM setter returned
  E_INVALIDARG.
  
  FreeRDP hit the identical gap in its own documentation-only enum until a
  two-line fix (FreeRDP#11035). Neither FreeRDP nor xrdp gate connection
  admission on this field at all; both decode it as a raw integer.
  Windows' own GetKeyboardType additionally documents 0x51 for generic HID
  keyboards, which a narrower fix adding only Korean would not survive.
  
  Marked as breaking since KeyboardType's public shape changes for any
  downstream consumer of ironrdp-pdu outside this workspace. Note that
  cargo-semver-checks in this repo's PR automation only runs against the
  facade ironrdp crate, which will not see a break confined to
  ironrdp-pdu's own API, so the automated breaking-change label may not
  apply here even though this is a real one.
  
  ## Validation
  
  cargo xtask check fmt/lints/tests/typos/locks all pass. Extended
  ironrdp-activex's existing inline COM boundary test to cover value 8
  round tripping and a negative value still being rejected.



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-viewer-v0.1.0)] - 2026-07-10

Initial release.
