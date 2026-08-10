# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-agent-v0.1.0...ironrdp-agent-v0.2.0)] - 2026-08-10

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

### <!-- 99 -->Please Sort

- Add agentic RDP CLI and localhost CI workflow ([#1289](https://github.com/Devolutions/IronRDP/issues/1289)) ([ffedb69ca8](https://github.com/Devolutions/IronRDP/commit/ffedb69ca8d4f2dec5a0649fa2cc4758ee74d13f)) 

- Extract reusable RDP daemon support ([#1543](https://github.com/Devolutions/IronRDP/issues/1543)) ([dc4692538e](https://github.com/Devolutions/IronRDP/commit/dc4692538e67ca089969879b7c62b69558e128e6)) 

- Add ActiveX RPC backend for ironrdp-agent ([#1544](https://github.com/Devolutions/IronRDP/issues/1544)) ([c31e43f755](https://github.com/Devolutions/IronRDP/commit/c31e43f755dbf24a302c86ca1ef8ba186d51216e)) 



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-agent-v0.1.0)] - 2026-07-10

Initial release.
