# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rpc-v0.2.0...ironrdp-rpc-v0.3.0)] - 2026-11-12

### <!-- 1 -->Features

- Log off the remote session through the NOW agent ([#2308](https://github.com/Devolutions/IronRDP/issues/2308)) ([d5e70f02b6](https://github.com/Devolutions/IronRDP/commit/d5e70f02b670dbe79ad7ef47203bfa82d32c16f4)) 

  Adds `ironrdp-agent session logoff` (alias `ironrdp-agent logoff`). It
  ends the user's session on the host instead of leaving it disconnected.
  Closes #1855.
  
  ## Why NOW and not RDP
  
  RDP has no way for a client to request a logoff:
  
  - The client's Shutdown Request is always denied while a user is logged
  on ([MS-RDPBCGR] 3.3.5.4.1).
  - MCS Disconnect Provider Ultimatum only disconnects.
  - `ERRINFO_LOGOFF_BY_USER` only reports a logoff after the host has done
  it.
  
  Devolutions Agent can log the user off from inside the session with
  `NOW_SESSION_LOGOFF_MSG`. It advertises `NOW_CAP_SESSION_LOGOFF` and
  handles the message with `ExitWindowsEx(EWX_LOGOFF)`.
  
  ## Changes
  
  - **`ironrdp-rpc`**: new `Request::Logoff` IPC request (tag 41).
  - **`ironrdp-daemon`**: advertises `NOW_CAP_SESSION_LOGOFF` and adds
  `OperationManager::session_logoff`. If the host has no NOW agent, or the
  agent doesn't support logoff, the request fails with an `unavailable`
  error and the session stays connected.
  - **`ironrdp-activex`**: the RPC host handles `Request::Logoff` the same
  way.
  - **`ironrdp-agent`**: the NOW logoff message has no response, so the
  command waits up to `--timeout-secs` (default 60) for the session to
  end. It succeeds only when the host ends the session with a logoff
  reason (`ERRINFO_LOGOFF_BY_USER` or `ERRINFO_RPC_INITIATED_LOGOFF`). It
  fails:
  - if the session ends for any other reason, since the user's session may
  still be running;
  - if the session is still connected after the timeout, saying an
  application may be blocking sign-out.
  
    Help text and README are updated.
  
  ## Dependencies
  
  - `now-client` is bumped to 0.2.0 and `now-proto-pdu` to 0.5, which add
  `NowClientHandle::session_logoff` (Devolutions/now-libraries#123).
  - `now-proto-pdu` 0.5 depends on `ironrdp-core` 0.2.1 from crates.io, so
  the lockfile carries that version next to the workspace's 0.3.1.
  
  ## Testing
  
  - New unit tests:
    - `Request::Logoff` round-trips through the IPC codec.
  - Against a mock daemon, the logoff command waits for the session to end
  and returns the host's reason.
  - `cargo clippy -D warnings` is clean for the touched crates, and `cargo
  xtask check fmt` passes.
  - Not tested yet against a live host running Devolutions Agent.
  
  ## Not included
  
  - A `--server` destination guard like `disconnect --server`.
  - A RAIL Execute `logoff.exe` fallback for RemoteApp sessions without
  Devolutions Agent.
  - Showing logoff support in `now capabilities` output. That needs a new
  field on the IPC `NowCapabilities` struct, which is a breaking change.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rpc-v0.1.0...ironrdp-rpc-v0.2.0)] - 2026-10-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-input` public dependency to 0.9

- [**breaking**] Update `ironrdp-rdpei` public dependency to 0.3
