# IronRDP TermSrv

Companion control-plane service for `ironrdp-wtsprotocol-provider`.

The companion accepts RDP connections, hands authenticated credentials to the Windows protocol provider, and captures the assigned Windows session through a session-local helper.
Keyboard, mouse, touch, and pen input return to that helper.
The control pipe carries listener lifecycle, connection, credential, and session-binding messages.
Clipboard redirection is opt-in.
Provider-mode desktop resizing uses the remote IDD.
Device redirection uses native TermService consumers through static-channel bridges.
RemoteApp/RAIL support is experimental and disabled by default.
Full native Terminal Server parity remains incomplete.

## Configuration

- Environment variable: `IRONRDP_WTS_CONTROL_PIPE`
  - Named pipe name (with or without `\\.\pipe\` prefix).
  - Default when unset: `IronRdpWtsControl`.

- Environment variable: `IRONRDP_WTS_LISTEN_ADDR`
  - TCP bind address for side-by-side incoming sockets.
  - Default when unset: `0.0.0.0:4489`.

- Environment variables: `IRONRDP_RDP_USERNAME`, `IRONRDP_RDP_PASSWORD`, `IRONRDP_RDP_DOMAIN`
  - Expected RDP credentials for standalone mode.
  - Provider mode uses Windows Negotiate for CredSSP/NLA and does not require a configured account or password.
    TermService authorizes the delegated credentials using normal Windows remote-logon policy.
  - Standalone mode requires `IRONRDP_RDP_USERNAME` and `IRONRDP_RDP_PASSWORD` for a successful CredSSP/NLA handshake.
  - Use down-level names as `IRONRDP_RDP_USERNAME=Administrator` with `IRONRDP_RDP_DOMAIN=IT-HELP`.
  - For UPNs, keep the full UPN in `IRONRDP_RDP_USERNAME` and leave `IRONRDP_RDP_DOMAIN` empty.
  - The interactive capture helper receives an allowlisted environment that excludes these credentials.

For Kerberos with `ironrdp-agent`, use the server's DNS name and `--prop kdcproxyurl:s:tcp://<kdc>:88` with a reachable KDC.
Kerberos negotiation and CredSSP TLS binding are verified.
Credential delegation also passes through an isolated build of the native acceptor, validated by a noninteractive Windows network logon.
Domain desktop logon remains unverified.

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
With the test-VM deployment script, pass `-Clipboard` to enable it in the scheduled companion process.

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

## RemoteApp

Set `IRONRDP_WTS_REMOTEAPP=1` in the companion's environment to enable native RemoteApp support in provider mode.
The default is `0`; other values and standalone mode are rejected.
RemoteApp requests are rejected when the opt-in is disabled.
Deploy a matching provider DLL and companion, and use a separate user session for RemoteApp.
The test-VM deployment script accepts `-WtsProvider -RemoteApp`, but does not deploy the provider DLL.
This is experimental support for published executable aliases.
Use `ironrdp-agent disconnect` when finished; closing the last RemoteApp window does not end its headless connection.

The provider selects the Windows system directory's `rdpinit.exe` as the authenticated session's initial program.
It authorizes this bootstrap with no parameters and published aliases through the [provider's application policy].
Windows' native RemoteApp shell executes the authorized application in the user's session.
Publish permitted applications through Windows RDS policy rather than disabling its allowlist.
The companion does not launch client-selected programs as SYSTEM or change the Windows application policy.
Neither the companion nor the provider starts Explorer for RemoteApp or before Client Info identifies a desktop connection.

The static `rail` channel carries native shell messages.
Native RAIL reads include reconstructed channel headers with `SHOW_PROTOCOL`; RDPDR and RDPSND reads remain unframed.
The native `railenc` endpoint is internal: its window orders become RDP graphics updates, not client DVC messages.
Window-order writes are serialized with bitmap updates and respect the client's fragmentation capabilities.
Only batches canceled before transmission may be retried.
Modern RAIL DVC and HiDef modes are disabled.
Private native property identifiers require compatibility checks when changing Windows builds.
`ironrdp-agent` reinitializes RAIL after Windows replaces `rdpshell.exe`, without reconnecting or replaying completed launches.
A restart with outstanding launch results still fails the connection because those results are ambiguous.
Windows can suppress the first post-restart launch when it matches an existing application, returning success without starting another process.
The client sends the current work area at initialization and reports display changes only when the dimensions change, avoiding redundant native desktop-hook restarts.
Controlled shell replacements preserve the application and connection on both native Windows and the provider listener.
This avoids an observed native shell crash trigger; the underlying Windows fault is not repaired.

[provider's application policy]: ../ironrdp-wtsprotocol-provider/README.md#published-remoteapp-applications

## Native dynamic channels

Provider mode bridges `Devolutions::Now::Agent` to the native NOW host in the assigned session.
Devolutions Agent must run `DevolutionsSession.exe` in that user session.
The companion transports NOW messages; it does not execute commands as SYSTEM or replace the desktop agent.
Provider mode also bridges `WebAuthN_Channel` (MS-RDPEWA), so `webauthn.dll` in the session reaches the client's authenticators.
Camera forwarding bridges `RDCamera_Device_Enumerator` and the device channels announced by the client after MS-RDPECAM version negotiation.
For the built-in client camera backend, select cameras with `camerastoredirect`.
Camera forwarding is experimental: native Windows capture is verified with generated RGB32, MJPEG, and H.264 frames, including native decoding, capture reopen, and reconnect.
H.264 checks cover constrained-baseline IDR/P frames and Main/High streams with B-frame reordering and moving content.
B-frame capture timestamps can be nonmonotonic on both this provider and native Windows RDP; direct Windows decoder tests reproduce this when timestamps follow decoding order rather than presentation order.
Playback timing and physical cameras remain unverified.
The RDP user's Windows camera privacy settings must allow capture.
TermService still authorizes native WTS opens.
Other native dynamic endpoints remain disabled.

Each connection permits one NOW open, four WebAuthn opens, one camera enumeration open, and four camera device opens at a time, each on a new DVC.
`webauthn.dll` opens a channel for each API call, so the WebAuthn limit bounds concurrent calls, not total calls.
The camera broker tracks at most 255 advertised devices, rejects names reserved for existing endpoints, and revokes their channels on device removal, enumeration closure, or session reassignment.
Open waits up to three seconds for peer acceptance and returns an error if negotiation, session binding, or transport is unavailable.
Session reassignment, disconnect, pipe closure, and queue overflow close the native channel.
A WTS caller closing its handle is a normal close and frees the open for reuse.
Messages are limited to 1 MiB, with at most 32 MiB of pending peer payloads, 32,768 queued messages, and one outstanding transport write per channel.
Camera device channels accept client messages up to 64 MiB plus the three-byte sample header, with the same byte limit for pending payloads; native-to-client messages retain the 1 MiB limit.
The limits admit a 16 MiB burst, which arrives in DVC messages of about 4 KiB.
Native reads and writes progress independently, so a blocked reader does not prevent the peer from sending output.
The private message pipe admits only SYSTEM and NetworkService; TermService transfers its handle to the authorized WTS caller.
Overlapped I/O preserves empty messages and message boundaries, including writes larger than the pipe buffer.
Native reads receive reconstructed `CHANNEL_PDU_HEADER` chunks of at most 1,600 payload bytes.

## Native device channels

Provider mode bridges the client's static `rdpdr` and `rdpsnd` channels to TermService's native `RDPDR` and `RDPSND` consumers.
Windows and the client implement the device protocols; the companion forwards their messages.
Both channels are needed because TermService does not start RDPDR communication without RDPSND.
The Agent uses a no-audio RDPSND backend, so successful device redirection does not establish audio playback support.

Native opens fail promptly when the client did not join the requested channel.
Each channel permits one native consumer at a time; reopening replaces the previous consumer without replaying stale messages.
Session reassignment and disconnect close the associated pipes.
Static messages may contain up to 16 MiB plus 64 KiB of protocol overhead, with at most 32 MiB of pending payloads and 32,768 queued messages per channel.
Reassembly is bounded even without a consumer, and fragments cannot cross consumer replacements.
This admits a 1 MiB drive read response including its 20-byte header without closing the redirected drive.
Queue overflow closes the channel rather than silently dropping device requests.
Only one native-to-client transport write is outstanding per channel.
Canceled queued writes are discarded; a write already in progress finishes to preserve RDP framing.
Static consumer reads use raw chunks of at most 1,600 bytes, without the dynamic bridge's `CHANNEL_PDU_HEADER`.
Logs contain channel names and message lengths, not payloads.

Use the [Agent filesystem configuration] to enable redirected drives.
For testing, expose a genuine isolated volume, not the host's system or data volumes.
The Windows backend rejects `subst` aliases under its reparse-point protections; use a mounted test ISO for read-only media or a temporary VHD for writable media.
If the build host cannot mount a VHD, run the Agent client inside the test VM and connect to its listener over loopback, redirecting only the isolated VHD.
Drive listing, large-file reads, creation, writes, rename, extension, truncation, and deletion are verified through native redirection.
Physical-smartcard interoperability remains unverified.

[Agent filesystem configuration]: ../ironrdp-agent/README.md#windows-filesystem-redirection

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
.\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1 -WtsProvider -CaptureIpc shm
```

Notes:

- Provider mode needs the deployment credentials only for remoting; it does not save a server-side RDP password.
- Standalone mode writes the configured RDP password to a restricted file on the VM, uses it once to start `ironrdp-termsrv`, then deletes it.
  The scheduled task definition never contains the password.
- The default host is `IT-HELP-TERMSRV`; both remoting and RDP use the environment-supplied identity.
- To use separate credentials, override `-Username`, `-RdpUsername`, `-PasswordEnvVar`, and `-RdpPasswordEnvVar`.
  Avoid plaintext password arguments in shell history.
- The startup task persists.
  Only standalone mode needs redeployment after a reboot to restore its one-use credential handoff.

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
