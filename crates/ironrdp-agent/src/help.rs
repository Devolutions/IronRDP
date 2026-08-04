//! The `--help-agent` guide: a concise, structured, LLM-friendly description of every operation.

/// Structured guide printed by `ironrdp-agent --help-agent`.
pub(crate) const AGENT_GUIDE: &str = r#"# ironrdp-agent

A CLI-driven, daemon-backed RDP client. One binary plays two roles:

- DAEMON: `ironrdp-agent daemon-start` runs a long-lived foreground process that owns the RDP
  engine and one RDP session. Background it yourself (e.g. `ironrdp-agent daemon-start &`).
- CLI: every other subcommand opens the local IPC endpoint, sends one request, prints the
  response, and exits.

The daemon stays alive across CLI invocations. One daemon serves one RDP session.

## Endpoint

Unix: `$XDG_RUNTIME_DIR/ironrdp-agent-<uid>.sock` (falls back to `/tmp/ironrdp-agent-<uid>.sock`).
Windows: `\\.\pipe\ironrdp-agent-<user>`.
Override with `--endpoint <PATH-OR-PIPE>` on any subcommand.

## Backends

- `--backend daemon` (default) uses `ironrdp-agent daemon-start` and its per-user endpoint.
- `--backend active-x` attaches to an already-hosted ActiveX control at its per-user
  `ironrdp-activex` endpoint. The host must set `IRONRDP_ACTIVEX_RPC=1` before creating the
  control; the agent never starts an ActiveX host. Use `--endpoint` when the host uses
  `IRONRDP_ACTIVEX_RPC_ENDPOINT`.

## Lifecycle

- `daemon-start [--overlay FILE] [--prop KEY:TYPE:VALUE]...`
                                 Start the daemon (foreground). Run this first. `--overlay`
                                 preloads a .rdp file as an overlay applied to every `connect`
                                 (overlay wins), letting an operator provision any setting out of
                                 band -- credentials in particular (e.g. the password). `--prop` is
                                 repeatable and layers additional overlay properties on top of
                                 `--overlay`, using the same `KEY:TYPE:VALUE` grammar as one .rdp
                                 file line (TYPE is `i` for integer or `s` for string), e.g.
                                 `--prop ironrdp_autologon:i:1`. Check `status` to see whether
                                 credentials are already loaded before supplying any yourself.
- `connect [--rdp-file F] [--prop KEY:TYPE:VALUE]... [--server H[:PORT]] [-u USER] [-p PASS] [-d DOMAIN] [--log-directive D]`
                                 Merge an optional .rdp file with CLI overrides into one config and
                                 open a session. Precedence (low to high): .rdp file -> `--prop`
                                 overrides -> named flags (`--server`/`-u`/`-p`/`-d`). When those
                                 flags are omitted, `RDP_HOSTNAME`, `RDP_USERNAME`, and
                                 `RDP_PASSWORD` supply their respective values; explicit flags
                                 override the environment. `--prop` is repeatable and lets you set
                                 any property without a dedicated flag existing for it, e.g.
                                 `--prop username:s:admin`. The selected backend validates the
                                 config and replies with an error listing any missing or invalid fields.
                                 If `status` reports `credentials loaded: true`, omit
                                 `-p/--password` (and any other preloaded secret) -- the backend
                                 supplies it. `--log-directive` refines this session's log capture
                                 (e.g. `ironrdp_connector=trace`) on top of the default `debug`
                                 level; use it to troubleshoot a connection, then read the result
                                 with `query-logs`.
- `disconnect`                   Tear down the current session (daemon keeps running).
- `status`                       Report connection state, destination, last frame size, and whether
                                 credentials are preloaded (`credentials loaded: true|false`). Query
                                 this first to decide whether you must supply a password.

## Inspection

- `query-props [--filter SUBSTR] [--prefix PREFIX]`
                                 Print the live session property bag, one `key = value` per line.
                                 Secrets are stripped from the configuration before a session
                                 starts, so the dump never contains passwords or tokens.
                                 `--filter` matches keys by substring; `--prefix` by prefix
                                 (both case-insensitive).
- `query-logs [--substring S] [--last N]`
                                 Print retained RDP session log lines (a bounded in-memory ring
                                 buffer, default level `debug`). `--substring` filters to matching
                                 lines; `--last N` keeps the last N. Raise verbosity for a specific
                                 session with `connect --log-directive`. This is the session's own
                                 log; the daemon's operational log goes to stderr (default `info`,
                                 tune with the `IRONRDP_LOG` env var).
- `screenshot [PATH]`            Capture the most recent frame (with the mouse cursor composited in)
                                 as a PNG and write it to PATH (default `screenshot.png`). Prints
                                 `wrote PATH (WxH, N bytes)`. Errors with `no frame available yet`
                                 until the first frame arrives.

## Input (require an active session)

- `mouse-move --x X --y Y`                       Move the pointer to an absolute position.
- `mouse-button --button <left|middle|right|x1|x2> --pressed <true|false>`
- `wheel --delta N [--horizontal]`               Rotate the wheel (negative N scrolls down/left).
- `key-scancode --scancode <0x1D|29> --pressed <true|false>`
- `key-unicode --char C --pressed <true|false>`  Type by Unicode character.
- `resize --width W --height H`                  Resize the remote desktop.

## NOW remote execution (requires an active, connected RDP session)

The daemon allocates one private `Devolutions::Now::Agent` DVC endpoint for each RDP session. It
waits lazily for the endpoint only when a NOW request is made: up to 30 seconds for its first
connection and up to 10 seconds after a worker/transport replacement. `status` and `disconnect`
remain responsive while it waits.

- `now capabilities`                 Negotiate and print the supported NOW styles.
- `now run COMMAND [--directory DIR]`
                                     Submit generic Run and return after local submission. Run is
                                     intentionally untracked: it has no durable output or result.
- `now powershell COMMAND [COMMON]`  Execute Windows PowerShell.
- `now pwsh COMMAND [COMMON]`        Execute PowerShell 7.
- `now exec process FILE [--parameters ARGS] [COMMON]`
                                     Execute a Windows CreateProcess request.
- `now exec batch COMMAND [COMMON]`  Execute a Windows batch request.

`COMMON` is `--directory DIR`, `--stdin FILE` (use `-` for the CLI standard input), `--timeout
SECONDS`, `--detached`, and `--operation-id-file FILE`. The operation-ID file is written after
local submission and lets later CLI invocations attach, cancel, or send stdin. PowerShell and pwsh
default to both `-NoProfile` and `-NonInteractive`; use `--profile` and/or `--interactive` only to
explicitly opt out. Detached commands have no stdin, output, or terminal result.

Tracked commands have one daemon-owned operation at a time. Their stdout and stderr chunks are
forwarded as raw bytes (not line-buffered) and the CLI returns the remote nonzero exit code
(1-255 directly; larger values as 255). Output is retained for `now attach`, `now list`, and `now
status`: 8 MiB per operation, 32 terminal operations, and 32 MiB total. Use:

- `now cancel OPERATION_ID`
- `now stdin OPERATION_ID --input FILE [--last]`
- `now attach OPERATION_ID [--after-sequence N]`
- `now list`
- `now status OPERATION_ID`
- `now diagnostics`

Live `now attach` output is bounded. If an attachment cannot keep up, it closes; attach again with
the last sequence number to resume from retained output.

Use `--format human|json|ndjson` with `now` for human-readable output, one JSON result, or JSON
event lines. JSON output represents raw bytes as byte arrays and is bounded to 8,192 events and
2 MiB of output. Use NDJSON for unbounded streaming.

Shell execution is intentionally not exposed: there is no `now shell` command, IPC request,
capability, or mapping, even if a peer advertises it.

## Errors

Failures print a single lowercase message (no trailing punctuation) and exit non-zero. A failed
`connect` carries the list of missing required fields.
"#;
