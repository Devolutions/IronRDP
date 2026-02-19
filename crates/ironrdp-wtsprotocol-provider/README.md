# IronRDP WTS Protocol Provider

Windows-specific Remote Desktop Services protocol provider implementation backed by IronRDP.

This crate is intended to host an `IWRdsProtocolManager`/`IWRdsProtocolListener`/`IWRdsProtocolConnection`
implementation for side-by-side protocol registration before any in-place replacement strategy.

## Status

Early implementation scaffold.

## Scope (current)

- Provider object model and session registry
- Listener lifecycle worker-thread dispatch
- Connection lifecycle state machine with transition validation
- CredSSP-first authentication bridge placeholders
- Input/video handle acquisition for keyboard, mouse, and video device paths
- Virtual channel endpoint creation via `IWRdsProtocolConnection::CreateVirtualChannel`
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
- WixSharp MSI project for `ceviche-rs` service packaging (`package/CevicheServiceWindowsManaged`)

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

`CreateVirtualChannel` now opens channel handles through `WTSVirtualChannelOpenEx` in the TermSrv process and returns those handles to the RDS stack.

This enables protocol-provider-side channel endpoint creation with proper lifecycle cleanup.

The provider now also recognizes and hooks known IronRDP server channel endpoints:

- Static virtual channels: `cliprdr`, `rdpsnd`, `drdynvc`
- Dynamic virtual channels: `Microsoft::Windows::RDS::DisplayControl`, `Microsoft::Windows::RDS::Graphics`, `FreeRDP::Advanced::Input`, `ECHO`

Hook behavior:

- Reuses existing opened handles by endpoint name (avoids duplicate opens for the same endpoint)
- Applies recommended dynamic priorities for known IronRDP dynamic channels when callers pass unknown/legacy priority values
- Ensures the `drdynvc` static backbone is opened before known IronRDP dynamic channels
- Starts per-channel forwarder workers for recognized IronRDP endpoints when a bridge handler is registered

## Virtual channel bridge handler API

The provider now exposes a process-wide bridge handler registration API:

- `set_virtual_channel_bridge_handler(...)`
- `VirtualChannelBridgeHandler`
- `VirtualChannelBridgeEndpoint`
- `VirtualChannelBridgeTx`

When registered, recognized IronRDP channel endpoints trigger:

1. `on_channel_opened(...)` with an endpoint descriptor and writable bridge tx.
2. `on_channel_data(...)` for payload bytes read from the TermSrv channel handle.
3. `on_channel_closed(...)` when the channel forwarder shuts down.

## Default named-pipe bridge (env-based)

If no custom bridge handler is registered, the provider can install a default named-pipe bridge automatically when this environment variable is set in the provider process:

- `IRONRDP_WTS_VC_BRIDGE_PIPE_PREFIX`

Behavior:

- One named-pipe worker is created per recognized channel endpoint.
- Pipe path format: `\\.\pipe\<prefix>.<svc|dvc>.<sanitized-channel-name>`
- Payload framing in both directions: little-endian `u32` length prefix + raw payload bytes.
- Provider → pipe: forwards payload bytes read from the TermSrv virtual channel handle.
- Pipe → provider: reads framed payloads from the pipe and forwards them back to the TermSrv virtual channel handle.

This makes it possible to stand up an external broker/service quickly using only the env var, while keeping custom in-proc bridge handler registration available for advanced scenarios.

However, there is no direct `IWRdsProtocolConnection*` callback API that streams virtual channel payload bytes into this provider implementation.

To route channel payloads all the way through external `ironrdp-server` channel processors, an additional broker layer is still required (for example, a bridge that reads/writes channel handles and forwards frames over an internal transport to `ironrdp-server`).

## Protocol manager CLSID

`{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}`

The install scripts use this CLSID by default and wire it into both:

- `HKLM\SOFTWARE\Classes\CLSID\{...}\InprocServer32`
- `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\<Listener>\LoadableProtocol_Object`

## Side-by-side rollout

See [SIDEBYSIDE_SETUP.md](./SIDEBYSIDE_SETUP.md).

For a Windows VM runbook that includes MSI service install/register/run and mstsc connection validation, see:

- `../../package/CevicheServiceWindowsManaged/README.md`
