# IronRDP WTS Protocol Provider

Windows-specific Remote Desktop Services protocol provider implementation backed by IronRDP.

This crate implements `IWRdsProtocolManager`, `IWRdsProtocolListener`, and `IWRdsProtocolConnection` for side-by-side protocol registration.

## Scope (current)

- Provider object model and session registry
- Provider-owned protocol manager/listener control flow (no built-in manager/listener delegation)
- Listener lifecycle worker-thread dispatch
- Optional provider/service control bridge over named-pipe IPC (`IRONRDP_WTS_CONTROL_PIPE`)
- Connection lifecycle state machine with transition validation
- CredSSP credential handoff from the companion service to Winlogon
- Caller-buffer licensing capability negotiation for the RDS grace-period path
- Input/video handle acquisition for keyboard, mouse, and video device paths
- Provider-owned native channel transport for NOW, WebAuthn, cameras, RDPDR, and RDPSND; unlisted endpoints are rejected
- In-proc COM class factory and DLL exports (`DllGetClassObject`, `DllCanUnloadNow`)
- Side-by-side install and uninstall PowerShell scripts
- Side-by-side preflight and registration verification PowerShell scripts
- Side-by-side backup/restore and firewall helper scripts
- One-command first-run orchestrator script and mstsc `.rdp` file generator
- Read-only side-by-side smoke test script for go/no-go checks before mstsc
- Provider DLL build helper and orchestrator auto-detect/build options
- Listener port conflict detection and port-aware firewall rule naming
- Optional post-restart readiness wait helper (`TermService` + TCP listen state)
- Backup manifests and rollback auto-detection of latest backup directory
- Remote Desktop enabled-state validation in preflight/smoke checks
- First-run package generator (ready-to-run install/rollback/connect artifacts)
- Executable package wrappers (`run-first-run-elevated.ps1`, `run-first-run.ps1`, `run-preview.ps1`, `run-preflight.ps1`, `run-install.ps1`, `run-install-restart.ps1`, `run-verify.ps1`, `run-smoke.ps1`, `run-connect.ps1`, `run-diagnostics.ps1`, `run-rollback.ps1`)
- CMD launchers for quick invocation (`run-first-run-elevated.cmd`, `run-first-run.cmd`, `run-connect.cmd`, `run-diagnostics.cmd`, `run-rollback.cmd`)
- Portable package archive output (`<package>.zip` + `<package>.zip.sha256.txt`)
- Diagnostics bundle collector for failed first-run attempts
- WixSharp MSI project for `ironrdp-termsrv` service packaging (`package/CevicheServiceWindowsManaged`)

## Session lifetime

Windows session arbitration can detach a connection from a temporary session before attaching it to an existing user session.
`DisconnectNotify` releases session-owned device handles but preserves the transport and connection callback for reassignment.
`Close` terminates the transport and makes the connection permanently unavailable.
The listener releases closed connections' callbacks on its next poll, without waiting for a companion `ConnectionBroken` event.
Windows still controls single-session policy and session selection.

CredSSP credentials are delivered through `GetClientData` and `GetUserCredentials`, not the listener's connection settings.
TermService converts connection settings to legacy buffers that accept only 20 username, 17 domain, and 14 password UTF-16 units; longer passwords otherwise fail before session creation.

## Input and graphics handle notes

`GetInputHandles` and `GetVideoHandle` now attempt to open host device handles instead of returning `NULL`.

Default probe order:

- Keyboard: `\\.\KeyboardClass0`, `\\.\KeyboardClass1`
- Mouse: `\\.\PointerClass0`, `\\.\PointerClass1`
- Video: `\\.\RdpVideoMiniport`, `\\.\DISPLAY`

Override environment variables (read by the provider process):

- `IRONRDP_WTS_KEYBOARD_DEVICE`
- `IRONRDP_WTS_MOUSE_DEVICE`
- `IRONRDP_WTS_VIDEO_DEVICE`

## Scope (next)

- Termsrv callback integration
- Input/video handle integration
- Licensing integration
- Virtual channel payload bridge from TermSrv channel handles to `ironrdp-server` channel processors

## Virtual channel routing status

`IWRdsProtocolConnection::CreateVirtualChannel` supports the dynamic NOW, WebAuthn, and camera endpoints and the static `RDPDR` and `RDPSND` endpoints through the companion's connection-local broker.
The experimental [RemoteApp opt-in] adds static `RAIL` and the internal `railenc` window encoder.
It requires a matching companion and provider DLL.
Initial-application authorization permits the trusted `rdpinit.exe` bootstrap without arguments and [published application aliases] on authenticated RemoteApp connections.
The query writes into TermService's caller-owned UTF-16 buffers, whose capacities are byte counts.
Metadata reads and native channel opens use the provider identity without changing the native RPC thread's impersonation token.
Connection-type queries identify RemoteApp sessions to TermService while desktop connections retain the native default.
Camera device names come from the client's MS-RDPECAM announcements, so the companion authorizes dynamic names against the connection's current devices rather than a fixed prefix.
Unlisted dynamic endpoints fail; unsupported static endpoints return `E_NOTIMPL`.
See the [native channel limits and camera status] before enabling redirection.
After peer acceptance, the provider opens a private overlapped message-pipe handle and transfers ownership to TermService.
TermService duplicates it into the authorized WTS caller and closes the original.
The companion reconstructs native read headers and closes the transport when the assigned session changes.

[native channel limits and camera status]: ../ironrdp-termsrv/README.md#native-dynamic-channels
[RemoteApp opt-in]: ../ironrdp-termsrv/README.md#remoteapp

Calling `WTSVirtualChannelOpenEx` from this callback recursively asks TermService to create the same channel, exhausting its threads and the VM's memory during logon.
Do not use WTS client APIs to implement the provider callback.

This restriction does not disable channel processors implemented directly in `ironrdp-termsrv`, such as its opt-in clipboard helper.

## Published RemoteApp applications

Publish applications through Windows RDS and request them as `||alias`.
The provider reads the existing `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Terminal Server\TSAppAllowList\Applications\<alias>` configuration.
It requires an absolute, unquoted `Path`; direct executable requests and unpublished aliases remain denied, except for the trusted bootstrap.
Global allow-unlisted-programs settings do not bypass these checks.
Connection Broker publication sources and document associations are not supported.

The [Windows publication properties] control arguments: `CommandLineSetting` 0 discards client arguments, 1 passes them through, and 2 replaces them with `RequiredCommandLine`.
A missing or empty `SecurityDescriptor` allows all authenticated RemoteApp users.
Otherwise, Windows `AccessCheck` checks read-data access against the RPC caller's impersonation token, never the provider's process token.
Malformed or unreadable policy fails the query rather than removing restrictions.

[published application aliases]: #published-remoteapp-applications
[Windows publication properties]: https://learn.microsoft.com/windows/win32/termserv/win32-tspublishedapplication

## Virtual channel bridge handler API

The provider retains a process-wide bridge handler registration API:

- `set_virtual_channel_bridge_handler(...)`
- `VirtualChannelBridgeHandler`
- `VirtualChannelBridgeEndpoint`
- `VirtualChannelBridgeTx`

The API defines these notifications:

1. `on_channel_opened(...)` with an endpoint descriptor and writable bridge tx.
2. `on_channel_data(...)` for incoming payload bytes.
3. `on_channel_closed(...)` when the channel closes.

Registration alone does not create a native transport, and `CreateVirtualChannel` does not currently issue these notifications.

## Default named-pipe bridge (env-based)

If no custom bridge handler is registered, the provider can install a default named-pipe bridge automatically when this environment variable is set in the provider process:

- `IRONRDP_WTS_VC_BRIDGE_PIPE_PREFIX`

The handler supports:

- One named-pipe worker is created per recognized channel endpoint.
- Pipe path format: `\\.\pipe\<prefix>.<svc|dvc>.<sanitized-channel-name>`
- Payload framing in both directions: little-endian `u32` length prefix + raw payload bytes.
- Provider → pipe: forwards payloads passed to `on_channel_data`.
- Pipe → provider: reads framed payloads and sends them through `VirtualChannelBridgeTx`.

Setting this variable does not enable native virtual channels.
The NOW broker uses its own connection-local transport, not this legacy handler.

## Protocol manager CLSID

`{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}`

The install scripts use this CLSID by default and wire it into both:

- `HKLM\SOFTWARE\Classes\CLSID\{...}\InprocServer32`
- `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\<Listener>\LoadableProtocol_Object`

## Side-by-side rollout

See [SIDEBYSIDE_SETUP.md](./SIDEBYSIDE_SETUP.md).

For a Windows VM runbook that includes MSI service install/register/run and mstsc connection validation, see:

- `../../package/CevicheServiceWindowsManaged/README.md`
