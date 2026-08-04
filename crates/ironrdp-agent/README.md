# IronRDP Agent

A CLI-driven RDP client designed for programmatic (e.g. LLM) consumption.

The `ironrdp-agent` binary is the CLI for the persistent daemon support:

- **Daemon** (`ironrdp-agent daemon-start`): a long-lived, foreground process implemented by
  [`ironrdp-daemon`] that owns the
  [`ironrdp-client`] engine and one RDP session. It stays alive across many CLI invocations and
  serves requests over a local IPC transport (a Unix domain socket on Unix, a named pipe on
  Windows).
- **CLI** (`ironrdp-agent <op> …`): a short-lived invocation that opens the IPC endpoint, sends a
  single request, prints the response, and exits.

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

## Secrets

The daemon never exposes secrets to the IPC reader. `ConfigBuilder::build` strips every
`ironrdp_cfg::is_secret_key` property (`ClearTextPassword`, `GatewayPassword`, the RDCleanPath
token, …) before producing the `Config`, and the daemon seeds its live property bag from that
post-build configuration. Secrets therefore never reach the live bag, so property dumps, status,
and logs cannot leak them — no separate redaction pass is needed.

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
.rdp file → --prop overrides → named flags (--server/--username/…) → daemon's overlay
```

On `connect`, `--prop` overrides win over an optional `--rdp-file` but lose to the named flags. On
`daemon-start`, `--prop` overrides win over an optional `--overlay` file, and the resulting overlay
still wins over everything a `connect` request supplies (unchanged).

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

After an RDP session connects, the daemon injects a per-session `Devolutions::Now::Agent` DVC
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

The ignored live test uses `IRONRDP_AGENT_E2E_HOST`, `IRONRDP_AGENT_E2E_USERNAME`,
`IRONRDP_AGENT_E2E_PASSWORD`, and optional `IRONRDP_AGENT_E2E_DOMAIN`:

```powershell
$env:IRONRDP_AGENT_E2E = '1'
cargo test -p ironrdp-agent --test live_e2e -- --ignored
```

[`ironrdp-client`]: ../ironrdp-client
[`ironrdp-core`]: ../ironrdp-core
[`ironrdp-daemon`]: ../ironrdp-daemon
[`ironrdp-propertyset`]: ../ironrdp-propertyset
[`ironrdp-viewer`]: ../ironrdp-viewer
