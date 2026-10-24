# IronRDP TermSrv

Companion control-plane service for `ironrdp-wtsprotocol-provider`.

The companion accepts RDP connections, hands authenticated credentials to the Windows protocol provider, and captures the assigned Windows session through a session-local helper.
Keyboard, mouse, touch, and pen input return to that helper.
The control pipe carries listener lifecycle, connection, credential, and session-binding messages.
Clipboard redirection is opt-in.
Provider-mode desktop resizing uses the remote IDD.
Audio and device redirection and full native Terminal Server parity are not yet implemented here.

## Configuration

- Environment variable: `IRONRDP_WTS_CONTROL_PIPE`
  - Named pipe name (with or without `\\.\pipe\` prefix).
  - Default when unset: `IronRdpWtsControl`.

- Environment variable: `IRONRDP_WTS_LISTEN_ADDR`
  - TCP bind address for side-by-side incoming sockets.
  - Default when unset: `0.0.0.0:4489`.

- Environment variables: `IRONRDP_RDP_USERNAME`, `IRONRDP_RDP_PASSWORD`, `IRONRDP_RDP_DOMAIN`
  - Expected RDP credentials used for authentication.
  - `IRONRDP_RDP_USERNAME` and `IRONRDP_RDP_PASSWORD` are required for a successful CredSSP/NLA handshake.
  - Use down-level names as `IRONRDP_RDP_USERNAME=Administrator` with `IRONRDP_RDP_DOMAIN=IT-HELP`.
  - For UPNs, keep the full UPN in `IRONRDP_RDP_USERNAME` and leave `IRONRDP_RDP_DOMAIN` empty.
  - The interactive capture helper receives an allowlisted environment that excludes these credentials.

Completed capture frames are read on a dedicated task so cancelled display polls cannot corrupt frame boundaries.

## Desktop cursors

The session helper captures native cursor shapes, hotspots, visibility, and application-driven position changes.
Alpha, transparency, and monochrome inversion are preserved through RDP pointer updates rather than painted into desktop frames.
Shapes up to 384 by 384 pixels use the client's negotiated pointer capabilities.
Cursor metadata shares the authenticated input connection in both TCP and shared-memory capture modes.
Native mouse-history markers distinguish injected input from application warps, so delayed movement echoes cannot rewind subsequent clicks.
Unavailable or oversized cursor images are logged and hidden until capture recovers; desktop capture and input continue.

## Touch and pen input

The session helper forwards RDPEI touch and pen input to Windows.
It supports contact cancellation, hover dismissal, touch geometry and pressure, and pen pressure, rotation, tilt, and eraser selection.
RDPEI 3.0 clients can use up to four in-range pen devices.
Frame intervals use native performance-counter timestamps unless the client disables timestamp injection.
Channel teardown releases tracked contacts and synthetic pen devices.
Input queues are bounded; overflow disconnects the client instead of silently dropping input.
Input waits through helper replacement and is enabled only after the helper can capture its desktop, not merely authenticate its IPC connection.

Touch and pen support is experimental.
Windows system touch gestures can consume multi-finger input before applications receive it.
To test ten-contact application delivery, temporarily turn off **Three- and four-finger touch gestures** in **Settings > Bluetooth & devices > Touch**, then restore the setting.
The companion does not change this preference.
An accepted client command does not prove delivery to an application.

## Desktop resizing

With IddCx 1.10 or newer, provider mode accepts the client's initial desktop size and single-monitor Display Control requests.
Initial dimensions are capped at 3840 by 2400 pixels.
Dynamic dimensions must be between 200 and 8192 pixels, with an even width and at most 9,216,000 pixels in total.
Rotation and per-monitor scaling are not applied.

Install the matching IDD package with `install-idd-remote.ps1` before starting the companion.
The installer protects `C:\ProgramData\IronRDPDisplay` so only SYSTEM and administrators can publish session-specific requests; the driver service accounts can read them.
The driver registers requested target modes before changing the Windows display configuration.
TCP and shared-memory capture report the actual desktop size, triggering RDP reactivation and capture-helper replacement.
Shared-memory objects allow only SYSTEM and the helper's user, with fresh names for each helper so resizing cannot reuse an old mapping.
An accepted client request alone does not prove that Windows applied it; inspect the resulting desktop dimensions and driver errors.

## Clipboard redirection

Set `IRONRDP_WTS_CLIPBOARD=1` in the companion's environment before starting it to enable bidirectional clipboard redirection.
The default is disabled (`0`); other values are rejected.
This is an experimental opt-in and does not yet enforce Windows RDS clipboard policy.

The RDP connection uses the existing CLIPRDR processor.
A separate helper runs the existing Windows clipboard backend in the user session assigned by TermService, never against session 0's clipboard.
It starts only when that session has a user token and is stopped when its connection ends or changes session.
Helper IPC uses an authenticated loopback connection, an eight-frame queue, and a 16 MiB frame limit.
Only one helper event may wait for server dispatch, including its RDP write.
Unavailable or oversized data produces a failed clipboard response rather than an empty successful paste.

Text, images, and registered clipboard formats use Windows delayed rendering.
Native format requests are serialized, with at most eight pending requests, so retries cannot reorder their responses.
File transfers use the Windows backend's OLE data objects and CLIPRDR file streams, including clipboard locks.
Up to 64 file requests may wait for a helper; an additional request fails the bridge and its pending requests.
Helper failure clears the remote clipboard offer and fails outstanding requests; reconnect to start a new helper.
No clipboard content is written to the companion's logs.

## Native dynamic channels

Provider mode bridges `Devolutions::Now::Agent` to the native Windows application in the assigned session.
The companion transports NOW messages; it does not execute commands as SYSTEM or replace the desktop agent.
TermService still authorizes native WTS opens.
Other native endpoints remain disabled.

Each connection permits one native NOW open at a time, with a new DVC for each reopen.
Open waits up to three seconds for peer acceptance and returns an error if negotiation, session binding, or transport is unavailable.
Session reassignment, disconnect, pipe closure, and queue overflow close the native channel.
Messages are limited to 1 MiB, with at most 4 MiB of pending peer payloads, 1,024 queued messages, and one outstanding transport write.
Native reads and writes progress independently, so a blocked reader does not prevent the peer from sending output.
The private message pipe admits only SYSTEM and NetworkService; TermService transfers its handle to the authorized WTS caller.
Overlapped I/O preserves empty messages and message boundaries, including writes larger than the pipe buffer.
Native reads receive reconstructed `CHANNEL_PDU_HEADER` chunks of at most 1,600 payload bytes.

## Build

```powershell
cargo build -p ironrdp-termsrv --release
```

## Run (debug)

```powershell
$env:IRONRDP_WTS_CONTROL_PIPE = "IronRdpWtsControl"
cargo run -p ironrdp-termsrv
```

## Deploy to a test VM (PSRemoting)

Supply `RDP_USERNAME` and `RDP_PASSWORD` through the environment, then run from the repository root:

```powershell
.\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1 -CaptureIpc shm
```

Notes:

- The deployment script does not embed the RDP password in the scheduled task definition.
  It writes the password to a restricted file on the VM, uses it once to start `ironrdp-termsrv`, then deletes it.
- The default host is `IT-HELP-TERMSRV`; both remoting and RDP use the environment-supplied identity.
- To use separate credentials, override `-Username`, `-RdpUsername`, `-PasswordEnvVar`, and `-RdpPasswordEnvVar`.
  Avoid plaintext password arguments in shell history.
- The startup task persists, but its credential handoff is consumed once.
  Redeploy after a later reboot to restore companion authentication credentials.

### Connect with ironrdp-agent

Use `ironrdp-agent`, not `mstsc` or MsRdpEx, to connect to this VM.
It reads `RDP_USERNAME` and `RDP_PASSWORD` directly, without password files or command-line secrets.
Native Windows RDP uses port 3389; the IronRDP side-by-side provider uses port 4489.

```powershell
.\crates\ironrdp-termsrv\scripts\connect-agent.ps1
ironrdp-agent status
```

Use `-Port 4489` for the custom provider.
Connections are asynchronous; inspect `status` before requesting a screenshot.
If TLS validation fails, verify the reported SHA-256 certificate fingerprint over authenticated PSRemoting before using `ironrdp-agent cert trust HOST:PORT SHA256`.
Pin the server certificate rather than disabling daemon-wide TLS checks.
For driver prerequisites and target-only signing trust, follow the [IDD setup instructions](../ironrdp-idd/README.md#set-up-it-help-termsrv).

## Local control-plane smoke test (no TermDD required)

You can validate the control-pipe command flow without `mstsc` by running `ironrdp-termsrv` and the smoke-test client under LocalSystem or NetworkService.
The pipe rejects interactive user accounts because it transports delegated credentials.

Example:

```powershell
$env:IRONRDP_WTS_CONTROL_PIPE = "IronRdpWtsControlLocal"
$env:IRONRDP_WTS_LISTEN_ADDR = "127.0.0.1:4496"
cargo run -p ironrdp-termsrv
```

In a second shell:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-control-pipe-local.ps1 \
  -PipeName IronRdpWtsControlLocal \
  -ListenerName IRDP-Tcp \
  -PortNumber 4496
```

Expected output includes:

- `listener_started`
- `incoming_connection`
- `connection_ready`
- `listener_stopped`
