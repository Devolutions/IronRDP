# IronRDP Agent

A CLI-driven, daemon-backed RDP client designed for programmatic (e.g. LLM) consumption.

The single `ironrdp-agent` binary bundles two roles:

- **Daemon** (`ironrdp-agent daemon-start`): a long-lived, foreground process that owns the
  [`ironrdp-client`] engine and one RDP session. It stays alive across many CLI invocations and
  serves requests over a local IPC transport (a Unix domain socket on Unix, a named pipe on
  Windows).
- **CLI** (`ironrdp-agent <op> …`): a short-lived invocation that opens the IPC endpoint, sends a
  request, prints the response, and exits. NOW execution keeps its local connection open to
  forward raw output chunks until the remote operation reaches a terminal result.

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

## Remote NOW execution

After connecting, run a command through the remote Devolutions NOW agent:

```text
ironrdp-agent now capabilities
ironrdp-agent now powershell '$PSVersionTable.PSVersion'
ironrdp-agent now pwsh --directory C:\work --timeout 60 '$PSVersionTable.PSVersion'
ironrdp-agent now pwsh --file ./script.ps1 --stdin ./input.bin
ironrdp-agent now exec process C:\Tools\tool.exe --parameters '--json'
ironrdp-agent now exec shell 'uname -a'
ironrdp-agent now exec batch 'dir /b'
```

`now capabilities` waits for the NOW DVC and prints the protocol version, named system/session
capabilities, and the execution capability intersection available to IronRDP Agent. It is the
recommended preflight for automation. The system and session entries are discovery-only in this
release; `powershell`, `pwsh`, `exec process`, `exec shell`, and `exec batch` are the exposed
execution operations. Each command is capability-gated before it is sent.

`powershell` and `pwsh` accept an inline command or a UTF-8 `--file` script. They use `-NoProfile`
and `-NonInteractive` by default; use `--profile` and/or `--interactive` to opt out. All tracked
execution modes accept `--directory`, `--timeout SECONDS`, `--stdin PATH` (or `--stdin -` for raw
local stdin), and `--operation-id-file PATH`. Standard output and standard error are forwarded
unchanged, as individual byte chunks, to the matching local stream. A nonzero remote exit code from
1 through 255 becomes the CLI process exit status; wider nonzero remote codes map to 255.

The CLI forwards the exit code carried by the terminal NOW result; it does not reinterpret a
PowerShell script's `exit` statement. A remote NOW implementation can map script failures before
returning its result code.

Output is not aggregated for the CLI, so long-running operations are not constrained by the former
15 MiB aggregate response limit. Each individual local IPC and NOW frame remains bounded at 16 MiB.

`--timeout` requests normal protocol cancellation with `NOW_EXEC_CANCEL_REQ` when the duration
expires. To cancel a command from another process, supply `--operation-id-file PATH` and run
`ironrdp-agent now cancel <OPERATION_ID>`. Cancellation waits for the remote terminal result to keep
the NOW byte stream synchronized. `--detached` is explicit, cannot be combined with stdin or a
timeout, and returns once the request has been written; it has no remote output or exit-status
tracking.

If the remote session does not open `Devolutions::Now::Agent`, the first command waits at most 30
seconds for its local DVC proxy endpoint, allowing delayed post-logon channel startup, then exits
with an error. Later reconnects wait at most 10 seconds. The daemon remains available for `status`
and `disconnect` while either wait is in progress.

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

[`ironrdp-client`]: ../ironrdp-client
[`ironrdp-core`]: ../ironrdp-core
[`ironrdp-propertyset`]: ../ironrdp-propertyset
[`ironrdp-viewer`]: ../ironrdp-viewer
