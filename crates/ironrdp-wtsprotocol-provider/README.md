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
- Provider-owned native NOW dynamic channel transport; other native endpoints are rejected
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
Windows still controls single-session policy and session selection.

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

`IWRdsProtocolConnection::CreateVirtualChannel` supports the dynamic `Devolutions::Now::Agent` endpoint through the companion's connection-local broker.
Other endpoints and static channels return `E_NOTIMPL`.
After peer acceptance, the provider opens a private overlapped message-pipe handle and transfers ownership to TermService.
TermService duplicates it into the authorized WTS caller and closes the original.
The companion reconstructs native read headers and closes the transport when the assigned session changes.

Calling `WTSVirtualChannelOpenEx` from this callback recursively asks TermService to create the same channel, exhausting its threads and the VM's memory during logon.
Do not use WTS client APIs to implement the provider callback.

This restriction does not disable channel processors implemented directly in `ironrdp-termsrv`, such as its opt-in clipboard helper.

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
