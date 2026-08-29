# IronRDP Agent

A CLI-driven RDP client designed for programmatic (e.g. LLM) consumption.

The `ironrdp-agent` binary is the CLI for the persistent daemon support:

- **Daemon** (`ironrdp-agent daemon-start`): a long-lived, foreground process implemented by
  [`ironrdp-daemon`] that owns the
  [`ironrdp-client`] engine and one RDP session. It stays alive across many CLI invocations and
  serves requests over a local IPC transport (a Unix domain socket on Unix, a named pipe on
  Windows).
- **Gateway forward** (`ironrdp-agent gw-forward`): a foreground listener that relays TCP through
  an RD Gateway without an RDP session or the daemon.
  `--socks5` serves SOCKS5 CONNECT (no auth); `--target HOST:PORT` is an SSH `-L`-style fixed forward.
  Credentials come from `--username`/`--password` or `RDG_USERNAME`/`RDG_PASSWORD`, falling back to
  `RDP_USERNAME`/`RDP_PASSWORD`.
  The listener defaults to `127.0.0.1`; do not expose unauthenticated SOCKS5 to untrusted networks.
- **CLI** (`ironrdp-agent <op> …`): a short-lived invocation that opens the IPC endpoint, sends a
  single request, prints the response, and exits.

Run `ironrdp-agent --help-agent` for a structured, machine-readable description of every operation.

## ActiveX backend

On Windows, an ActiveX host can expose its session through the same local RPC protocol. Start the
host with `IRONRDP_ACTIVEX_RPC=1`, then use `--backend active-x` for agent operations. The agent
uses the per-user `ironrdp-activex` endpoint by default and never attempts to start an ActiveX host.
Use `--endpoint` when the host selected `IRONRDP_ACTIVEX_RPC_ENDPOINT`.
The RAIL audit commands require the daemon backend.

`connect` accepts `RDP_HOSTNAME`, `RDP_USERNAME`, and `RDP_PASSWORD` as defaults for its named
connection flags. Explicit flags override those process-local values. The native MSTSC bridge uses
these only when it is explicitly enabled; `RDP_AUTOLOGON` is active only when its value is exactly
`1`, and requires nonempty username and password values.

## Windows filesystem redirection

On Windows, start the daemon with one or more `--rdpdr-drive NAME=VOLUME_ROOT` options to opt in to static filesystem redirection.
For example, `ironrdp-agent daemon-start --rdpdr-drive System=C:\ --rdpdr-drive Data=D:\` exposes two local volumes to every session created by that daemon.
Each root must be a unique existing local volume root in the exact `C:\` form, and each protocol-visible name must be unique case-insensitively and contain at most seven ASCII letters, numbers, spaces, underscores, hyphens, periods, or a trailing colon.
The configured drives are fixed for the daemon lifetime; hot-plug and rescan are not supported.

## Windows smartcard redirection

On Windows, enable WinSCard smartcard redirection with `daemon-start --smartcard`, overlay/connect property `ironrdp_smartcard:i:1`, or a sandbox config with `SmartCardRedirection` enabled.
Smartcard can be enabled without redirected drives (smartcard-only RDPDR).
Connect-time `ironrdp_smartcard:i:0` disables it for that session even when the daemon was started with `--smartcard`.

## TLS certificate validation

The daemon performs strict certificate and hostname validation by default.
For an explicitly authorized test endpoint, start the daemon with `--skip-certificate-check`.
This startup-only flag accepts any certificate and server name, so it is vulnerable to on-path attacks and is unavailable through `connect`.

## Headless RemoteApp validation

The daemon-backend `rail` commands expose client-validated RAIL handshake, launch, and window-order evidence without rendering a RemoteApp UI or accepting raw protocol PDUs.
Connect in RemoteApp mode with `remoteapplicationmode:i:1` and a canonical `remoteapplicationprogram:s:<program>` property.
The client queues that program as the initial RAIL Execute request after the server handshake and keeps Client Info Alternate Shell and Working Directory empty.
An `alternate shell` value is only used as a compatibility fallback when `remoteapplicationprogram` is absent.
Start `ironrdp-agent daemon-start` in another terminal before connecting.
The target must publish and allow the requested RemoteApp.

```powershell
ironrdp-agent connect --server rdp.example.test --prop remoteapplicationmode:i:1 --prop remoteapplicationprogram:s:notepad.exe
ironrdp-agent rail status
ironrdp-agent rail --format ndjson events
ironrdp-agent rail execute notepad.exe --arguments C:\Temp\audit.txt
```

`rail events` retains 256 observations per connection generation.
Sequences remain monotonic across resize reconnects even though each new generation starts with fresh history.
When a caller resumes too far behind, the returned stream contains a `gap` event reporting the last sequence number no longer retained.
Set `N` to the latest sequence observed; `rail wait --after-sequence N --timeout-ms 30000` returns retained later observations immediately or waits up to 30 seconds without polling.
If an accepted local launch fails before it is sent, the event stream reports `execute_failed` with a stable reason and no working-directory or argument data.
Use `rail --format json` for one deterministic document or `rail --format ndjson` for one event per line.
The agent advertises no local RAIL shell-integration flags because it does not implement move/size, taskbar, cloak, z-order, or display-power behavior.

## Bounded Unicode input

`type-unicode --text TEXT` sends at most 96 Unicode characters in ordered FastPath input events.
The daemon reserves all queue slots before changing keyboard state, so queue backpressure sends none of a rejected request.
The ActiveX backend explicitly rejects this bulk input operation.

## Hyper-V VMConnect

On Windows, the daemon can connect to a Hyper-V VM console through the host's VMConnect endpoint on port 2179.
Use `--vmconnect-current-user` for native SSPI authentication with the caller's Windows logon token.
The local host defaults to `localhost`, so this path needs neither `--server` nor a reusable password.

```powershell
ironrdp-agent daemon-start
ironrdp-agent connect --vmconnect <VM_ID> --vmconnect-basic --vmconnect-current-user
ironrdp-agent screenshot vm-console.png
```

List VM IDs with `Get-VM | Select-Object Name, Id`.
The basic console accepts Hyper-V's private frame-buffer DVC and copies the host-created shared-memory DIB into the daemon's retained frame.
Omit `--vmconnect-basic` for Enhanced Session mode.
Omit `--vmconnect-current-user` and supply host credentials when integrated authentication is not appropriate.

## Windows Sandbox

On Windows with the Windows Sandbox feature enabled, the agent can create and attach to a sandbox over the product's default **named-pipe** transport (`\\.\pipe\{VMId}`).
The connection uses standard RDP security with no encryption (`PROTOCOL_RDP` / `ENCRYPTION_LEVEL_NONE`).

```bat
:: create, inspect, and stop via WindowsSandboxServer gRPC
ironrdp-agent sandbox start
ironrdp-agent sandbox list
ironrdp-agent sandbox config <sandbox-id>

:: connect (daemon must already be running)
ironrdp-agent daemon-start
ironrdp-agent connect --sandbox-id <sandbox-id>
ironrdp-agent screenshot sandbox.png

:: shut down when finished
ironrdp-agent sandbox stop <sandbox-id>
```

The agent speaks `sandboxserver.SandboxCore` in-process over the per-user named pipe (`\\.\pipe\wsandbox\<md5(user SID)>`) — no .NET helper is required.
WindowsSandboxServer must already be running; opening the Sandbox UI or invoking `wsb` starts it.
On retail builds that permit one active sandbox, stop that initial sandbox before using `sandbox start`; the agent reports server policy errors rather than bypassing them.
`sandbox start` accepts `--id <GUID>` and `--config <FILE>`, reads the same configuration XML accepted by `wsb start --config`, and prints the created sandbox Id.

Low-level escape hatch when you already have the pipe path and guest password:

```bat
ironrdp-agent connect --sandbox-pipe \\.\pipe\{VMId} -u WDAGUtilityAccount -p <password>
```

NamedPipe remains the default Windows Sandbox transport.
Use VMConnect for Hyper-V VMs rather than replacing the Sandbox recipe.

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
cargo test -p ironrdp-testsuite-extra --test integration_tests_extra -- --ignored agent::live_e2e
```

The RemoteApp variant uses `IRONRDP_AGENT_RAIL_E2E=1` plus the corresponding
`IRONRDP_AGENT_RAIL_E2E_HOST`, `IRONRDP_AGENT_RAIL_E2E_USERNAME`, `IRONRDP_AGENT_RAIL_E2E_PASSWORD`, and optional `IRONRDP_AGENT_RAIL_E2E_DOMAIN` variables.
It starts an isolated daemon with `--skip-certificate-check`, so run it only against an explicitly authorized test endpoint.

[`ironrdp-client`]: ../ironrdp-client
[`ironrdp-core`]: ../ironrdp-core
[`ironrdp-daemon`]: ../ironrdp-daemon
[`ironrdp-propertyset`]: ../ironrdp-propertyset
[`ironrdp-viewer`]: ../ironrdp-viewer
