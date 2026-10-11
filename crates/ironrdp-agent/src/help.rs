//! The `--help-agent` guide: a concise, structured, LLM-friendly description of every operation.

/// Structured guide printed by `ironrdp-agent --help-agent`.
pub(crate) const AGENT_GUIDE: &str = r#"# ironrdp-agent

A CLI-driven, daemon-backed RDP client. One binary plays three roles:

- DAEMON: `ironrdp-agent daemon start` starts a background process that owns the RDP engine
  and one RDP session. `connect` starts the default daemon automatically when it is absent.
- GATEWAY: `ironrdp-agent gw-forward` runs in the foreground and relays TCP through an RD Gateway
  without an RDP session. It does not use the daemon or IPC.
- CLI: every other subcommand opens the local IPC endpoint, sends one request, prints the
  response, and exits.

The daemon stays alive across CLI invocations. One daemon serves one RDP session.

For automation, always pass `connect --no-prompt`, even in SSH, tmux, or other TTY environments.
It returns after the request is accepted; poll `status` for the outcome instead of waiting for a human prompt.

## Endpoint

Unix: `$XDG_RUNTIME_DIR/ironrdp-agent-<uid>.sock` (falls back to `/tmp/ironrdp-agent-<uid>.sock`).
Windows: `\\.\pipe\ironrdp-agent-<user>`.
Override with `--endpoint <PATH-OR-PIPE>` on any subcommand.

## Backends

- `--backend daemon` (default) uses the per-user daemon endpoint.
- `--backend active-x` attaches to an already-hosted ActiveX control at its per-user
  `ironrdp-activex` endpoint. The host must set `IRONRDP_ACTIVEX_RPC=1` before creating the
  control; the agent never starts an ActiveX host. Use `--endpoint` when the host uses
  `IRONRDP_ACTIVEX_RPC_ENDPOINT`.
  RAIL audit commands and terminal `attach` require the daemon backend.

## Lifecycle

- `daemon start [--foreground] [--overlay FILE] [--prop KEY:TYPE:VALUE]... [--skip-certificate-check] [--rdpdr-drive NAME=VOLUME_ROOT]... [--smartcard]`
                                 Start the default daemon in the background and wait for readiness.
                                 `--foreground` runs it in this terminal instead; `daemon-start`
                                 remains a legacy foreground alias.
                                 An existing daemon is reused only when no startup options are specified.
                                 A losing concurrent startup fails; inspect `daemon status` before retrying.
                                 Background logs: `%LOCALAPPDATA%\ironrdp-agent\daemon.log` on Windows,
                                 `$XDG_STATE_HOME/ironrdp-agent/daemon.log` on Unix (default `~/.local/state`).
- `daemon status` / `daemon list` / `daemon stop`
                                 Inspect or stop the default daemon independently of its RDP session.
                                 `list` covers the selected per-user endpoint, not arbitrary custom endpoints.
                                 `stop` closes the session and exits; `disconnect` only ends the RDP session.
- `session list` / `session disconnect [--server HOST[:PORT]]`
                                 List the one active session on the selected daemon, or disconnect it
                                 and wait for termination. The daemon keeps running.
                                 `--server` is a safety check against disconnecting a different
                                 destination, not a way to select among multiple sessions.
- `daemon-start [--overlay FILE] [--prop KEY:TYPE:VALUE]... [--skip-certificate-check] [--rdpdr-drive NAME=VOLUME_ROOT]... [--smartcard]`
                                 Legacy foreground alias for `daemon start --foreground`. `--overlay`
                                 preloads a .rdp file as an overlay applied to every `connect`
                                 (overlay wins), letting an operator provision any setting out of
                                 band -- credentials in particular (e.g. the password). `--prop` is
                                 repeatable and layers additional overlay properties on top of
                                 `--overlay`, using the same `KEY:TYPE:VALUE` grammar as one .rdp
                                 file line (TYPE is `i` for integer or `s` for string), e.g.
                                 `--prop ironrdp_autologon:i:1`. Check `status` to see whether
                                 credentials are already loaded before supplying any yourself.
                                 On Windows, repeat `--rdpdr-drive NAME=VOLUME_ROOT` to opt in to
                                 static filesystem redirection. Each root must be a unique existing
                                 local volume root in the exact `C:\` form, and each one-to-seven-character
                                 ASCII drive name must be unique (case-insensitive). The configured
                                 set is fixed for the daemon lifetime; drive hot-plug and rescan are
                                 not supported. On Windows, `--smartcard` enables WinSCard smartcard
                                 redirection (same as overlay/connect `ironrdp_smartcard:i:1`).
                                 Sandbox connects with `SmartCardRedirection` also set that property
                                 at connect time, which can enable smartcard without `--smartcard`.
                                 TLS certificate and hostname validation is strict by default.
                                 `--skip-certificate-check` disables both for this daemon only.
                                 Use it only for an explicitly authorized test endpoint because it accepts any certificate and is vulnerable to on-path attacks.
                                 Prefer pinning one server's certificate with `cert trust`.
- `cert trust ENDPOINT SHA256` / `cert list` / `cert remove ENDPOINT`
                                 Manage the known-certificates store (no daemon required).
                                 An interactive `connect` prompts to accept an untrusted
                                 certificate once or always (`--no-prompt` disables it; it never
                                 prompts without a terminal).
                                 When strict validation fails, `status` reports the certificate's
                                 SHA-256 fingerprint and the exact `cert trust` command. A pinned
                                 certificate is accepted only for that HOST:PORT (default 3389);
                                 a different certificate fails again. Takes effect on the next
                                 `connect`. Verify the fingerprint out of band before trusting it.
                                 Path: `%APPDATA%\ironrdp-agent\known_certificates` (Windows) or
                                 `~/.config/ironrdp-agent/known_certificates`; override with
                                 `IRONRDP_AGENT_KNOWN_CERTIFICATES`.
- `connect [--no-prompt] [--no-auto-start|--auto-start] [--rdp-file F] [--prop KEY:TYPE:VALUE]... [--server H[:PORT]] [-u USER] [-p PASS] [-d DOMAIN] [--vmconnect VM_ID] [--vmconnect-basic] [--vmconnect-current-user] [--sandbox-id ID] [--sandbox-pipe PATH] [--log-directive D]`
                                 Merge an optional .rdp file with CLI overrides into one config and
                                 open a session. The default daemon starts automatically when absent;
                                 `--no-auto-start` requires it to be running. An explicit
                                 `--endpoint` needs `--auto-start` to launch a daemon there;
                                 `--backend active-x` never auto-starts.
                                 Always use `--no-prompt` for automation; it returns after the request is accepted.
                                 Otherwise a daemon-backed TTY waits up to 120 seconds for the outcome and may prompt to trust a certificate.
                                 Precedence (low to high): .rdp file -> `--prop`
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
                                 On Windows, `--sandbox-id` resolves NamedPipe RDP settings via
                                 WindowsSandboxServer gRPC (create the VM first with `wsb start`).
                                 Sandbox defaults are the base; explicit file/prop/flags override
                                 them, except NamedPipe TLS/CredSSP stay forced off. Prefer
                                 `--sandbox-id` over `--sandbox-pipe`; the pipe escape hatch needs
                                 `-u`/`-p` (guest password from `sandbox config`).
                                 On Windows, `--vmconnect VM_ID` routes the session through the
                                 Hyper-V host on port 2179. `--vmconnect-basic` selects the basic
                                 console. `--vmconnect-current-user` uses native SSPI with the
                                 caller's logon token, needs no username or password, and defaults
                                 an omitted server to localhost. Local VMConnect accepts the private
                                 frame-buffer DVC and reads its shared-memory DIB.
- `gw-forward --gateway HOST[:PORT] (--socks5 | --target HOST:PORT) [--listen ADDR]`
                                 Forward TCP through an RD Gateway without an RDP session.
                                 `--socks5` serves SOCKS5 CONNECT (no auth); `--target` is an
                                 SSH `-L`-style fixed forward. Credentials come from
                                 `--username`/`--password` or `RDG_USERNAME`/`RDG_PASSWORD`,
                                 falling back to `RDP_USERNAME`/`RDP_PASSWORD`.
                                 The listener defaults to `127.0.0.1`; do not expose unauthenticated SOCKS5 to untrusted networks.
- `disconnect [--server HOST[:PORT]]`  Alias for `session disconnect`; the optional server
                                 must match the active destination before any disconnect occurs.
- `status`                       Report connection state, destination, last frame size, and whether
                                 credentials are preloaded (`credentials loaded: true|false`). Query
                                 this first to decide whether you must supply a password.

## Inspection

  RAIL commands require the daemon backend.
  Connect with `--prop remoteapplicationmode:i:1`; add
  `--prop remoteapplicationprogram:s:<program>` to queue an initial launch.

- `rail status`                     Show RAIL handshake and synchronization state plus agent-queued
                                   launches.
- `rail events [--after-sequence N]`
                                   Show validated RAIL observations retained by the daemon.
- `rail wait [--after-sequence N] [--timeout-ms MS]`
                                   Return retained RAIL events newer than `--after-sequence`, waiting
                                   up to the timeout only when none are available.
- `rail execute EXECUTABLE [--working-directory DIR] [--arguments ARGS] [--flags FLAGS]`
                                   Queue a bounded, validated RemoteApp launch.
  All `rail` subcommands accept `--format human|json|ndjson` before the subcommand.
  `json` prints one JSON document; `ndjson` prints one JSON object per returned event or response.
  RAIL event history is bounded to 256 records per connection generation and returns an explicit
  `gap` event after eviction.
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
- `screenshot --terminal [--protocol auto|sixel|kitty|iterm2] [--columns N]`
                                 Render the frame inline in the terminal instead of writing a file.
                                 `auto` detects the protocol from the environment; `--columns`
                                 caps the width (default: terminal width).
- `attach [--protocol P] [--interval-ms N] [--cell-size WxH] [--no-fit]`
                                 Interactive, for humans: show the live session in the terminal
                                 and forward mouse and keyboard input. The footer has clickable
                                 [Menu], [Fit], and [Detach] controls. Menu options also accept
                                 1-5 and Esc; input stays local while the menu is open.
                                 Option 5 disconnects only after y or a confirmation click;
                                 n/Esc cancels without ending the session.
                                 Ctrl+] detaches; Ctrl+\ fits the desktop. Auto-fit is on by default;
                                 `--no-fit` disables it, and the menu toggles it for this attachment.
                                 Detaching keeps the RDP session running; confirmed Disconnect
                                 ends the RDP session but leaves the daemon running.
                                 Session termination exits the view even while the menu is open; resize reconnections do not.
                                 Requires the daemon backend; ActiveX attachment is unsupported.
                                 Requires a TTY; agents should use `screenshot` and input commands.

## Input (require an active session)

- `mouse-move --x X --y Y`                       Move the pointer to an absolute position.
- `mouse-button --button <left|middle|right|x1|x2> --pressed <true|false>`
- `wheel --delta N [--horizontal]`               Rotate the wheel (negative N scrolls down/left).
- `key-scancode --scancode <0x1D|29> --pressed <true|false>`
- `key-unicode --char C --pressed <true|false>`  Type by Unicode character.
- `type-unicode --text TEXT`                     Type at most 96 Unicode characters all-or-nothing.
- `touch --x X --y Y --action <down|move|up|out-of-range|cancel|hover>
    [--contact-id N] [--encode-time MS] [--frame-offset US]`
                                                 Send one MS-RDPEI touch contact sample.
- `touch-tap --x X --y Y [--contact-id N]`       DOWN then UP at the same point via RDPEI.
- `touch-frame --contact id:x:y:action [...]`    One multi-contact MS-RDPEI touch frame.
- `pen --x X --y Y --action <down|move|up|out-of-range|cancel|hover>
    [--device-id N] [--pressure N] [--rotation N] [--tilt-x N] [--tilt-y N]
    [--eraser] [--inverted] [--encode-time MS] [--frame-offset US]`
                                                 Send one MS-RDPEI pen contact sample.
- `pen-tap --x X --y Y [--device-id N] [--pressure N]`
                                                 DOWN then UP pen tap via RDPEI.
- `dismiss-hovering [--contact-id N]`            Dismiss a hovering touch contact.
- `resize --width W --height H`                  Resize the remote desktop.

## Clipboard

Text (`CF_UNICODETEXT`), images (`CF_DIB`/`CF_DIBV5`, as PNG files), HTML fragments
(`HTML Format`), and files (the `FileGroupDescriptorW` file-list mechanism); no folders. Local
content is a single logical item: setting one replaces whatever was set before, regardless of kind.
A remote copy is requested files over image over HTML over text, richest representation first,
when the remote offers more than one. File listing is metadata only; a file's contents are fetched
only on explicit request.

- `clipboard-get`                    Print the last text received from the remote clipboard, or
                                      `(empty)` if none has arrived yet. Requires an active session.
- `clipboard-set --text TEXT`        Set the local clipboard text and advertise it to the remote.
                                      Works before a session connects too: the text is remembered
                                      and advertised as soon as the clipboard channel initializes.
- `clipboard-get-image [PATH]`       Write the last image received from the remote clipboard to
                                      `PATH` (default `clipboard.png`) as a PNG, or print a
                                      no-image message if none has arrived yet.
- `clipboard-set-image PATH`         Set the local clipboard image from the PNG at `PATH` and
                                      advertise it to the remote. Same before-connect behavior as
                                      `clipboard-set`.
- `clipboard-get-html`               Print the last HTML fragment received from the remote
                                      clipboard, or `(empty)` if the current remote item isn't
                                      HTML (nothing has arrived, or the last copy was text, an
                                      image, or files).
- `clipboard-set-html --html HTML`   Set the local clipboard HTML fragment and advertise it to the
                                      remote. Same before-connect behavior as `clipboard-set`.
- `clipboard-set-files PATH...`      Offer one or more local files to the remote via the clipboard
                                      file-list mechanism. Each path must be a regular file; a
                                      directory is rejected outright, not skipped. Works before a
                                      session connects too: the offer is stored and advertised as
                                      soon as the clipboard channel initializes. Once a session is
                                      active, it must have negotiated file transfer support, or the
                                      call fails.
- `clipboard-list-files`             List the remote's currently offered files (name, path within
                                      the copied collection, size, last-write time as Unix seconds,
                                      and whether it is a directory entry), or a no-files message if
                                      none are offered. Nothing is downloaded; this only inspects
                                      metadata already received.
- `clipboard-get-file INDEX --out PATH`
                                      Fetch one file's full contents by its position in the last
                                      `clipboard-list-files` listing and write it to `PATH`. Fails
                                      cleanly on a directory entry, an out-of-range index, or a
                                      file too large for the RPC transport, rather than attempt a
                                      partial or corrupted download.

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

## Windows Sandbox (Windows only)

Start the Windows Sandbox UI or `wsb` once to bootstrap WindowsSandboxServer.
The agent can then create, inspect, and stop sandboxes over its per-user named pipe without a .NET helper.
On retail builds that permit one active sandbox, stop that initial sandbox before using `sandbox start`; the agent reports server policy errors rather than bypassing them.

- `sandbox start [--id GUID] [--config FILE]`
                                  Start a sandbox and print its Id.
                                  The server uses its default configuration when `--config` is omitted.
- `sandbox list`                 List running sandbox Ids via WindowsSandboxServer.
- `sandbox config <ID>`          Print a redacted RdpClientConfig summary (password shown as set/empty).
- `sandbox stop <ID>`            Shut down a running sandbox via gRPC.
- `connect --sandbox-id <ID>`    Fetch config + connect over `\\.\pipe\{VMId}` (PROTOCOL_RDP /
                                 ENCRYPTION_LEVEL_NONE). Daemon must already be running.
- `connect --sandbox-pipe PATH -u USER -p PASS`
                                 Low-level NamedPipe connect when you already have the guest password.

Default product transport is NamedPipe.
VMConnect is available for Hyper-V VMs; NamedPipe remains the Windows Sandbox default.

## Errors

Failures print a single lowercase message (no trailing punctuation) and exit non-zero. A failed
`connect` carries the list of missing required fields.
"#;
