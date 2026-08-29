# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-agent-v0.1.0...ironrdp-agent-v0.2.0)] - 2026-08-29

### <!-- 0 -->Security

- Restrict Windows named pipe to the current user ([#1482](https://github.com/Devolutions/IronRDP/issues/1482)) ([26821836b7](https://github.com/Devolutions/IronRDP/commit/26821836b78e0670f4a0a41eb7021f6e1c256be4)) 

  ## Summary
  
  The `ironrdp-agent` Windows named-pipe listener inherited the pipe
  namespace's default ACL, so **any local user** could connect to a
  running daemon and drive the session — inject input, capture
  screenshots, read logs, and trigger NOW remote execution. The Unix
  listener already locks its socket to `0o600` (owner-only); this mirrors
  that stance on Windows.
  
  ## The fix
  
  Build a **protected DACL** (`D:P(A;;GA;;;{user-sid})`) granting
  `GENERIC_ALL` only to the current user's SID, and apply it to **every**
  `CreateNamedPipeW` instance (both the first in `bind` and the
  replacement minted in `accept`) via tokio's
  `create_with_security_attributes_raw`. The descriptor is cached on the
  `Listener` and reused across instances (the kernel copies it on each
  `CreateNamedPipeW`, so it only needs to outlive each synchronous call).
  
  Implementation follows the established windows-FFI style used by sibling
  crates (`ironrdp-cliprdr-native`): the `windows` crate (0.62, matching
  siblings), RAII guards (`OwnedTokenHandle`, `OwnedSecurityDescriptor`)
  for `CloseHandle`/`LocalFree` cleanup, one unsafe op per block with `//
  SAFETY:` comments, `.cast::<>()` + `try_from` instead of `as` casts, and
  `tracing::warn!` on cleanup failure.
  
  ## Why this matters
  
  On shared Windows hosts the `\\.\pipe\` namespace is reachable by every
  local user by default. This is the last line of defense against a
  different local user taking over a running agent session. It closes the
  asymmetry with the Unix path, which already documents the `/tmp`
  fallback as world-writable and explicitly sets `0o600`.
  
  ## Test coverage
  
  `transport.rs` previously had **zero tests**. Added a Windows-gated
  `#[cfg(test)]` smoke test
  (`security_descriptor_for_current_user_is_non_null`) that exercises the
  full token → SID → SDDL → security-descriptor pipeline for the current
  process and asserts a non-null descriptor is produced — the happy path
  `Listener::bind` depends on. (Asserting the ACL denies a *different*
  user requires a second token context and is out of scope.)
  
  ## Verification
  
  All `cargo xtask` CI-equivalent checks pass on a clean tree:
  
  - `cargo xtask check fmt -v` → All good
  - `cargo xtask check lints -v` (workspace clippy `--all-targets -D
  warnings`) → All good (ironrdp-agent produces zero warnings)
  - `cargo test -p ironrdp-agent` → 12 passed; 0 failed (incl. the new
  test)
  - `cargo xtask check locks -v` → All good
  
  ## Files
  
  - `crates/ironrdp-agent/Cargo.toml` — add `windows = "0.62"`
  (cfg(windows),
  Win32_Foundation/Security/Security_Authorization/System_Threading)
  - `crates/ironrdp-agent/src/transport.rs` —
  `OwnedTokenHandle`/`OwnedSecurityDescriptor` RAII guards,
  `create_server_instance` helper, `Listener` caches + reuses the SD; new
  smoke test
  - `Cargo.lock` — +1 line registering `windows` as an ironrdp-agent dep
  (no new versions; crates already present via siblings)
  
  ## Release
  
  This is a `fix(agent):` conventional commit, picked up by release-plz
  into the next release cycle (alongside the existing `feat(agent)`
  NOW-integration commit) to propose the `0.2.0` bump once merged to
  `master`. CHANGELOG is auto-generated — no manual edit needed.

- Connect to Windows Sandbox named pipes ([#1580](https://github.com/Devolutions/IronRDP/issues/1580)) ([39b020343d](https://github.com/Devolutions/IronRDP/commit/39b020343d962962bfbefc89939be64d5c716196)) 

  Windows Sandbox's default attach path is a local named pipe carrying
  plain TPKT/X.224 with PROTOCOL_RDP and ENCRYPTION_LEVEL_NONE, not
  TCP:3389 or VMConnect. Allow the connector and client to complete that
  sequence only via an explicit opt-in (`enable_standard_rdp_security`;
  NamedPipe enables it), and teach ironrdp-agent to resolve pipe path and
  guest credentials from WindowsSandboxServer after `wsb start`.
  
  Adds Transport::NamedPipe, ironrdp_named_pipe/ironrdp_sandbox_id
  properties, sandbox list/config/stop CLI helpers via an in-process
  h2/gRPC client on the per-user `\\.\pipe\wsandbox\{guid}` pipe (no .NET
  helper), and connect --sandbox-id / --sandbox-pipe. Sandbox-derived
  properties are the merge base; explicit .rdp/--prop/flags override them
  while NamedPipe TLS/CredSSP stay forced off. Local :2179+PCB remains
  unsupported.

### <!-- 1 -->Features

- Integrate NOW client ([#1451](https://github.com/Devolutions/IronRDP/issues/1451)) ([405770e2a6](https://github.com/Devolutions/IronRDP/commit/405770e2a6232f506e10f88ab6cb413d6bec7c5b)) 

  ## Summary
  - Integrate the released registry `now-client = "0.1.0"` into
  `ironrdp-agent`; no Git, path, patch, or vendored dependency is used.
  - Add a per-session `Devolutions::Now::Agent` DVC endpoint with
  30-second initial readiness and 10-second reconnect deadlines.
  - Add durable NOW operations, IPC streaming/retention, supported CLI
  forms (`run`, `powershell`, `pwsh`, `exec process`, `exec batch`), safe
  PowerShell defaults, raw output forwarding, and remote exit propagation.
  - Add IPC/retention coverage and an adapter regression proving immediate
  Run frames are quarantined before a following Process execution.
  - Do not modify Gateway or `ironrdp-dvc-pipe-proxy`.
  
  ## Validation
  - `cargo test -p ironrdp-agent`
  - `cargo test -p ironrdp-testsuite-extra --test integration_tests_extra
  agent`
  - `cargo clippy -p ironrdp-agent --all-targets -- -D warnings`
  - `cargo xtask check fmt -v`
  - `cargo xtask check tests -v`
  - `cargo xtask check locks -v`
  
  `cargo xtask check lints -v` is blocked by a pre-existing `ironrdp-str`
  `manual_is_multiple_of` lint under the available Rust 1.90 toolchain.
  `cargo xtask check typos -v` cannot run because `typos-cli` is not
  installed. Authorized-VM end-to-end validation remains pending access to
  the designated RDP/NOW endpoint.
  
  ---------

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

- Configure static RDPDR drives ([#1617](https://github.com/Devolutions/IronRDP/issues/1617)) ([b06cd71e0f](https://github.com/Devolutions/IronRDP/commit/b06cd71e0f2845893db2123c8280f4a80e1b5a36)) 

  Allow an agent daemon to opt in to fixed Windows filesystem drives.
  
  Validate names, roots, and duplicate definitions before listening, then
  attach the native RDPDR backend factory to each client.

- Add authorized RDPDR harness ([#1620](https://github.com/Devolutions/IronRDP/issues/1620)) ([24eb1f62f9](https://github.com/Devolutions/IronRDP/commit/24eb1f62f9aadf36fb6b3b588ecda390fcad2b38)) 

  Add an opt-in Windows harness that verifies \\tsclient direct PowerShell
  and Explorer copy paths for an explicitly authorized endpoint.
  
  Keep TLS certificate and hostname validation strict by default, but
  permit a daemon-start-only bypass for the authorized test endpoint. Add
  bounded all-or-nothing Unicode text input for remote commands without
  expanding ActiveX.

- Add RAIL audit commands ([#1646](https://github.com/Devolutions/IronRDP/issues/1646)) ([77759dca03](https://github.com/Devolutions/IronRDP/commit/77759dca032eb829b5be54a3c44d9be92252cf41)) 

  Expose bounded client-validated RAIL events and RemoteApp launch
  requests through the daemon IPC so headless agents can verify sessions.
  
  Preserve cursors across resize reconnects, wake waiting readers for
  locally queued launches, and redact launch data in logs.
  
  Report terminal local Execute failures without disrupting an otherwise
  valid RDP session.

- Send MS-RDPEI touch over RPC ([#1651](https://github.com/Devolutions/IronRDP/issues/1651)) ([254fa5565d](https://github.com/Devolutions/IronRDP/commit/254fa5565dc82b9f9deadf037ecd044cdea537e3)) 

  Expose Request::Touch so ironrdp-agent can inject RDPEI contact frames
  through the daemon and ActiveX backends for live testing.
  
  CLI touch/touch-tap map to legal contact flag sets, and ActiveX rejects
  illegal combinations before queueing.

- Expose MS-RDPEI pen and multi-touch ([#1652](https://github.com/Devolutions/IronRDP/issues/1652)) ([51f8738ea1](https://github.com/Devolutions/IronRDP/commit/51f8738ea156daeea292c63bba09d9137791af57)) 

  Extend the agent RPC path beyond single-contact touch so multi-contact
  frames, pen frames, and dismiss-hovering can be driven end-to-end
  against a real host.
  
  Wire Pen/Dismiss through rpc, daemon, CLI, ActiveX control RPC, client
  input loop, and session encode helpers. Add pen contact flag legality
  checks and round-trip coverage.

- Enable Windows smartcard in product surfaces ([#1672](https://github.com/Devolutions/IronRDP/issues/1672)) ([1561a4c327](https://github.com/Devolutions/IronRDP/commit/1561a4c327512a1a3a6c618d4b06dc388fab132f)) 

  Wire daemon, agent, viewer, and ActiveX to the WinSCard RDPDR backend so
  products can announce a smartcard device only with a matching factory.
  
  Expose `WindowsRdpdrBackendFactory::with_smartcard`, daemon/agent
  `--smartcard` and `ironrdp_smartcard` connect overrides, viewer
  `--smartcard`, and ActiveX `RedirectSmartCards`. Smartcard-only RDPDR is
  valid on Windows; non-Windows paths refuse or hard-disable enablement.

- Start Sandbox VMs over gRPC ([#1671](https://github.com/Devolutions/IronRDP/issues/1671)) ([f3fdb5d565](https://github.com/Devolutions/IronRDP/commit/f3fdb5d5653166bbdd4d3f8aaec26846e2d2d2ec)) 

  Create Windows Sandbox instances through the SandboxCore named pipe.
  Accept an optional GUID and configuration file.
  Wait up to two minutes for provisioning before reporting failure.
  Print the created ID and report single-instance policy failures.

- Forward TCP through an RD Gateway tunnel ([#1714](https://github.com/Devolutions/IronRDP/issues/1714)) ([ac7faecb33](https://github.com/Devolutions/IronRDP/commit/ac7faecb338bb9a7f2f4a097ee8a2d85f944ccfb)) 

  Forward TCP through an RD Gateway without an RDP session.
  
  MS-TSGU is not RDP-specific. ironrdp-agent gw-forward exposes an SSH
  -L-style local port forward or a SOCKS5 CONNECT proxy, opening one
  WebSocket gateway tunnel per inbound connection with
  GwClient::connect_with_port.
  
  Master GwClient is WebSocket and HTTP Basic only, so this slice does not
  add an RPC transport.
  
  Local-to-gateway copies are capped at 8182 bytes so each write fits in
  the 8192-byte MS-TSGU data packet. SOCKS5 carries explicit RFC 1928
  reply codes, rejects a non-zero reserved byte, and does not send a
  CONNECT reply after a method-negotiation failure. --socks5 and --target
  are exclusive, and --target is parsed with TargetAddr so IPv6 requires
  an explicit port and is passed to the gateway unbracketed.
  GwClient::poll_shutdown is a no-op on master, so a local half-close is
  not forwarded to the target.

- Add smart-card gateway authentication ([#1741](https://github.com/Devolutions/IronRDP/issues/1741)) ([761dae12b0](https://github.com/Devolutions/IronRDP/commit/761dae12b0128be0f9bae52ca59eae8a0ba0b02f)) 

  Add an opt-in Kerberos PKINIT path for HTTP Negotiate gateway
  authentication while retaining the password Negotiate, NTLM, and Basic
  flows.
  
  The public credentials type accepts an application-supplied UPN, redacts
  credentials, and rejects unsupported smart-card feature or challenge
  combinations without exposing them.

- Expose Hyper-V VMConnect ([#1770](https://github.com/Devolutions/IronRDP/issues/1770)) ([e1e2cafea7](https://github.com/Devolutions/IronRDP/commit/e1e2cafea73290e116b348dab3a3864cfe597657)) 

  Expose Hyper-V VMConnect through the agent CLI and daemon build. Add
  VMConnect mode and current-user options, default an omitted host to
  localhost, and enable the client VMConnect feature in the daemon.

### <!-- 4 -->Bug Fixes

- Preserve RAIL audit correlation ([#1655](https://github.com/Devolutions/IronRDP/issues/1655)) ([fe0e86d814](https://github.com/Devolutions/IronRDP/commit/fe0e86d8144364c9d8bcd0597be2372d5bbdf9cf)) 

  Retain RAIL audit history after a session terminates while clearing
  Execute requests that can no longer receive a result, including
  connection failures.
  Render daemon RAIL errors in the selected machine format and exit
  nonzero.
  Keep bounded RAIL event encoding symmetric with decoding.

### <!-- 99 -->Please Sort

- Add agentic RDP CLI and localhost CI workflow ([#1289](https://github.com/Devolutions/IronRDP/issues/1289)) ([ffedb69ca8](https://github.com/Devolutions/IronRDP/commit/ffedb69ca8d4f2dec5a0649fa2cc4758ee74d13f)) 

- Extract reusable RDP daemon support ([#1543](https://github.com/Devolutions/IronRDP/issues/1543)) ([dc4692538e](https://github.com/Devolutions/IronRDP/commit/dc4692538e67ca089969879b7c62b69558e128e6)) 

- Add ActiveX RPC backend for ironrdp-agent ([#1544](https://github.com/Devolutions/IronRDP/issues/1544)) ([c31e43f755](https://github.com/Devolutions/IronRDP/commit/c31e43f755dbf24a302c86ca1ef8ba186d51216e)) 



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-agent-v0.1.0)] - 2026-07-10

Initial release.
