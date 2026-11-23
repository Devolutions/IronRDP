# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rpc-v0.2.0...ironrdp-rpc-v0.3.0)] - 2026-11-23

### <!-- 0 -->Security

- Verify local IPC peer is the same user on connect ([#2483](https://github.com/Devolutions/IronRDP/issues/2483)) ([63d442e8dd](https://github.com/Devolutions/IronRDP/commit/63d442e8dd2baa3ae2ee048f19ba116d40fd813a)) 

  The local IPC client connects to a predictable, per-user endpoint — a
  named pipe on Windows, a Unix socket under `$XDG_RUNTIME_DIR` (or
  `/tmp`). The listener restricts the endpoint to the current user, but
  until the daemon binds it, another local user can create the endpoint
  first and answer the client.
  
  `connect()` now verifies the peer is owned by the current user before
  returning, so no request is sent to a foreign-owned endpoint:
  
  - **Windows**: the listener sets the pipe object's owner to the current
  user (SDDL `O:`); the client reads the connected pipe's owner via
  `GetSecurityInfo` and compares it to its own token SID (`EqualSid`). A
  normal user cannot create a kernel object owned by another user's SID,
  so an impostor's pipe never matches. The client QoS is pinned to
  `SECURITY_IDENTIFICATION` so a server cannot impersonate the client.
  - **Unix**: compare `peer_cred()` uid to the effective uid.
  
  A mismatch, or any failure to confirm ownership, fails closed.
  Legitimate same-user connections are unaffected.

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

- Number frames and add wait-frame ([#2418](https://github.com/Devolutions/IronRDP/issues/2418)) ([aa969ce6a7](https://github.com/Devolutions/IronRDP/commit/aa969ce6a7970cf5ec51d81228de6d5527255c3e)) 

  A caller driving a session can't tell whether the picture changed
  between two requests, and can't wait for it to change. `status` reports
  only the frame size, and `screenshot` returns the latest frame whether
  or not it's new, so a script has to poll screenshots and compare pixels
  to learn that the server drew something.
  
  The daemon now counts the frames it receives in a session. `status`
  reports the count as `frame: N`, and the new `wait-frame --after-frame N
  [--timeout-ms MS]` prints the same report once a frame with a greater
  number arrives. The caller reads N before the action whose result it
  waits for, so a frame that arrives before the wait starts still
  satisfies it. A wait that ends by its timeout or by the session ending
  is an error, so success always means a newer frame.

- Report server session facts ([#2437](https://github.com/Devolutions/IronRDP/issues/2437)) ([555f872f0f](https://github.com/Devolutions/IronRDP/commit/555f872f0ff4413c64f4b03bf7c3f4d333e20461)) 

  Retain and expose server-reported session ID, logon status and error,
  network characteristics, and monitor layout through `session info` in
  human, JSON, or NDJSON format.
  
  Capture connect-time network measurements, clear facts when a server
  redirects or a fallback resize starts a new connection, preserve them
  across automatic reconnects, and report session information as
  unavailable in the ActiveX backend.

### <!-- 4 -->Bug Fixes

- Keep session facts current and complete ([#2447](https://github.com/Devolutions/IronRDP/issues/2447)) ([c759493afa](https://github.com/Devolutions/IronRDP/commit/c759493afaee87659baa5b8d9010c2545d4845b3)) 

  When an automatic reconnect ends in `Connected`, the server rejected the
  cookie and logged on afresh, so the facts reported before belong to a
  session this connection didn't return to. The daemon now notes what the
  server reports from the moment a reconnect begins and keeps only that
  when the reconnect ends this way. A reconnect that ends in
  `AutoReconnected` keeps everything as before. After a rejected cookie
  the monitor layout reads empty until the server sends one, because the
  client passes on the layout from connection setup only on a first
  connection.
  
  A Network Characteristics Result carries baseRTT or bandwidth only when
  its request type says so (MS-RDPBCGR 2.2.14.1.5). A result that leaves
  one out now keeps the value reported before, in the connector's
  connect-time measurement and in the daemon's session facts, instead of
  erasing it. averageRTT is in every result and always replaces.
  
  The docs now also say that a resize falling back to a new connection
  clears the facts, which it already did. The client builds its
  connect-time event in place of a single-use helper, and the resize
  fallback's facts check has a test of its own.

### <!-- 6 -->Documentation

- Drop session info auto-reconnect claims ([#2460](https://github.com/Devolutions/IronRDP/issues/2460)) ([6457c5fd28](https://github.com/Devolutions/IronRDP/commit/6457c5fd28259182a151aeffb892fdc3b24b3f24)) 

  The daemon builds its client without automatic reconnect, so the agent's
  help and README and the SessionInfo docs described a cookie-based
  reconnect, kept or rejected, that the daemon never makes. They now
  describe only the redirect and resize-fallback cases.
  
  The daemon still tracks facts across an automatic reconnect, so that
  code holds if the daemon enables it, and its invariant is now written
  down on since_reconnecting. The network field cites MS-RDPBCGR
  2.2.14.1.5 for the base RTT and bandwidth a result can leave out.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rpc-v0.1.0...ironrdp-rpc-v0.2.0)] - 2026-10-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-input` public dependency to 0.9

- [**breaking**] Update `ironrdp-rdpei` public dependency to 0.3
