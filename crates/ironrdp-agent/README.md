# IronRDP Agent

A CLI-driven RDP client designed for programmatic (e.g. LLM) consumption.

The single `ironrdp-agent` binary provides a short-lived CLI and can drive one of three
long-lived local RPC backends:

- **Daemon** (`ironrdp-agent daemon-start`): a long-lived, foreground process that owns the
  [`ironrdp-client`] engine and one RDP session. It stays alive across many CLI invocations and
  serves requests over a local IPC transport (a Unix domain socket on Unix, a named pipe on
  Windows).
- **Viewer** (`ironrdp-viewer --rpc`): the same RPC contract hosted by the visible
  [`ironrdp-viewer`]. The viewer window and CLI share one RDP session, framebuffer, and input path.
- **ActiveX** (`ironrdpax.dll`): the same RPC contract hosted by an opted-in ActiveX control.
  Set `IRONRDP_ACTIVEX_RPC=1` in the host process before creating the control, then select it with
  `--backend active-x`. This backend is never auto-started because the control must remain hosted
  by its owner.

For normal CLI operations, the selected backend is started automatically when it is not already
running and remains available for later invocations. The daemon is the default; select the visible
viewer with `--backend viewer`, or an already-hosted control with `--backend active-x`. `--endpoint`
overrides the selected backend's endpoint. Use `stop` to terminate the selected backend; for
ActiveX, it stops only the listener and leaves the hosted session running.

The CLI (`ironrdp-agent <op> …`) is a short-lived invocation that opens the selected endpoint, sends
a single request, prints the response, and exits.

Run `ironrdp-agent --help-agent` for a structured, machine-readable description of every operation.

## Prebuilt binaries

Prebuilt, checksummed archives are attached to each GitHub Release under the `ironrdp-agent-v*`
tags. See the [Releases page](https://github.com/Devolutions/IronRDP/releases) for per-platform
download and verification instructions.

## Wire format

Messages are encoded with [`ironrdp-core`]'s `Encode`/`Decode` traits, length-delimited with a
little-endian `u32` byte-count prefix. There is no JSON anywhere. Both ends are the same binary at
the same version, so the format carries no version byte.

Connection configuration travels as a binary-encoded [`PropertySet`][`ironrdp-propertyset`] inside a
strictly-typed `Request::Connect`. Runtime operations (mouse, keyboard, status, logs, …) are
strictly-typed messages. `Request::Screenshot` returns the most recent frame as PNG bytes (with the
mouse cursor composited in — the agent enables software pointer rendering), which the CLI writes to
disk.

All backends implement the complete current RPC contract, including connection lifecycle, status,
property and log inspection, screenshots, mouse/keyboard/resize input, and NOW operations. The
default endpoints are `ironrdp-agent-<uid>.sock` (Unix) or `\\.\pipe\ironrdp-agent-<user>` (Windows)
for the daemon, and the corresponding `ironrdp-viewer-<uid>.sock` or
`\\.\pipe\ironrdp-viewer-<user>` endpoint for the viewer. A manually started viewer uses
`ironrdp-viewer --rpc --rpc-endpoint <PATH-OR-PIPE>` when a non-default endpoint is required.
The ActiveX endpoint is `ironrdp-activex-<uid>.sock` (Unix) or
`\\.\pipe\ironrdp-activex-<user>` (Windows), and can be overridden in the host process with
`IRONRDP_ACTIVEX_RPC_ENDPOINT`.

## Secrets

The RPC backends never expose secrets to the IPC reader. `ConfigBuilder::build` strips every
`ironrdp_cfg::is_secret_key` property (`ClearTextPassword`, `GatewayPassword`, the RDCleanPath
token, …) before producing the `Config`, and daemon and ActiveX RPC sessions seed their live
property bags from that post-build configuration. Direct ActiveX COM sessions use a separate
secret-free snapshot. Secrets therefore never reach the live bag, so property dumps, status, and
logs cannot leak them — no separate redaction pass is needed.

## Preloaded overlay

An operator can preconfigure any settings — credentials in particular — without handing them to the
IPC caller. Pass an overlay [`PropertySet`][`ironrdp-propertyset`] to `daemon-start --overlay FILE`;
the daemon layers it on top of every `Request::Connect` before building the configuration (overlay
wins). When the overlay carries a secret (password/token), `Request::Status` reports
`credentials_loaded`, so a caller should check the status first to learn whether it still needs to
supply a password.

## Property overrides

`connect` and `daemon-start` both accept a repeatable `--prop KEY:TYPE:VALUE` flag, using the same
grammar as one `.rdp` file line (`TYPE` is `i` for integer or `s` for string, e.g.
`--prop ironrdp_autologon:i:1 --prop username:s:admin`). It lets a caller set any property without a
dedicated CLI flag existing for it. Final precedence, low to high:

```
.rdp file → --prop overrides → environment-backed or named flags (--server/--username/…) → daemon's overlay
```

On `connect`, `RDP_HOSTNAME`, `RDP_USERNAME`, and `RDP_PASSWORD` provide defaults for `--server`,
`--username`, and `--password`. Explicit CLI flags override their corresponding environment value;
both override `--prop` and an optional `--rdp-file`. On `daemon-start`, `--prop` overrides win over
an optional `--overlay` file, and the resulting overlay still wins over everything a `connect`
request supplies (unchanged).

## Logging

Two logging concerns are kept separate:

- **Daemon logging** is the daemon's own operational logging (IPC handling, lifecycle). It is the
  global `tracing` subscriber: a compact formatter writing to stderr, defaulting to `info` and
  tunable with the `IRONRDP_LOG` environment variable, mirroring [`ironrdp-viewer`].
- **RDP session logging** is captured into a small, queryable in-memory ring buffer (read via
  `Request::QueryLogs`) instead of the terminal. It is installed as a thread-local subscriber for
  the session thread only (`tracing::dispatcher::with_default`), so it never becomes the global
  subscriber. It defaults to `debug`; a per-`Connect` `log_directive` (e.g. `ironrdp_connector=trace`)
  refines the filter to troubleshoot IronRDP itself.

## NOW remote execution

After an RDP session connects, the daemon support injects a per-session `Devolutions::Now::Agent` DVC
endpoint. The endpoint is local to the daemon and is not shared between RDP sessions. NOW protocol
framing, negotiation, heartbeats, capability gates, and command PDU handling are owned by the
`now-client` dependency; `ironrdp-daemon` owns only the endpoint/reconnect boundary and durable operation
state.

Use the supported commands below after `connect`:

```shell
ironrdp-agent now capabilities
ironrdp-agent now run "notepad.exe"
ironrdp-agent now powershell "Get-Process"
ironrdp-agent now pwsh "Get-Date"
ironrdp-agent now exec process cmd.exe --parameters "/c echo hello"
ironrdp-agent now exec batch "echo hello"
```

`now run` is deliberately untracked and reports only local submission. Process, Batch, PowerShell,
and pwsh commands are daemon-tracked unless `--detached` is supplied. Tracked raw stdout and stderr
chunks are streamed without line buffering; a terminal nonzero exit code is returned by the CLI
(1–255 directly, wider values as 255). `now cancel`, `now stdin`, `now attach`, `now list`, and
`now status` operate on daemon-owned operation IDs. Initial stdin comes from `--stdin FILE` (or
`-`); live stdin is bounded to 1 MiB per request. Use `--operation-id-file FILE` to persist an
operation ID immediately after local submission for a later attach, cancellation, or stdin call.

PowerShell and pwsh use `-NoProfile` and `-NonInteractive` by default. `--profile` and
`--interactive` are explicit opt-outs. The agent retains at most 8 MiB of output per operation, 32
terminal records, and 32 MiB across retained records. `now attach --after-sequence N` replays
bounded output then follows a running operation. Live attachments are bounded and disconnect when
they cannot keep up; attach again with the last sequence number to resume from retained output. Use
`now --format human|json|ndjson` for raw human streaming, a JSON result, or JSON event lines; JSON
represents output bytes as arrays and is bounded to 8,192 events and 2 MiB of output. Use NDJSON
for unbounded streaming.

The local DVC endpoint is connected only on a NOW request. Its first readiness deadline is 30
seconds; a replacement after a worker/transport failure has a 10-second deadline. No Shell command,
IPC request, capability, or execution mapping is exposed, even when a remote peer supports Shell.

## Localhost RDP workflow

The manual `.github/workflows/agentic-rdp.yml` workflow builds `ironrdp-agent`, enables localhost
RDP on a Windows runner, connects at the requested desktop size, drives the desktop through the
agent, and uploads logs and screenshots. Run the same scenario from an elevated Windows shell with:

```powershell
cargo build -p ironrdp-agent --release
.\testing\agentic-rdp\Invoke-AgenticRdpTest.ps1 -DesktopSize 1920x1080
```

The script temporarily changes local RDP settings and the current user's password. Use it only on a
disposable test machine.

## Live end-to-end test

The ignored live test uses the normal `RDP_HOSTNAME`, `RDP_USERNAME`, `RDP_PASSWORD`, and optional
`RDP_DOMAIN` environment variables. It remains ignored by default, so it runs only when explicitly
selected:

```powershell
cargo test -p ironrdp-agent --test live_e2e -- --ignored
```

[`ironrdp-client`]: ../ironrdp-client
[`ironrdp-core`]: ../ironrdp-core
[`ironrdp-propertyset`]: ../ironrdp-propertyset
[`ironrdp-viewer`]: ../ironrdp-viewer
