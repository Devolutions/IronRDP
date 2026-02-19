# Side-by-side setup (first-run prep)

If you also want to install and run the companion `IronRdpCevicheService` via MSI in a Windows VM before mstsc validation, see:

- `package/CevicheServiceWindowsManaged/README.md`

This crate currently provides an initial Windows protocol-provider scaffold using:

- `IWRdsProtocolManager`
- `IWRdsProtocolListener`
- `IWRdsProtocolConnection`

The intended rollout is side-by-side first (new protocol listener name), before any in-place replacement of the built-in protocol.

## Current status

- COM interface implementations are present.
- In-proc COM activation exports are present (`DllGetClassObject`, `DllCanUnloadNow`).
- Protocol manager/listener delegation to built-in Windows RDP is removed.
- Listener startup dispatches `OnConnected` from a dedicated worker thread.
- With control IPC available, listener worker polls companion-service incoming
	connections and dispatches `OnConnected` per accepted socket.
- Without control IPC configured, listener worker falls back to a bootstrap connection dispatch
	for sequencing validation.
- Optional control-plane IPC bridge to companion service is available via
	`IRONRDP_WTS_CONTROL_PIPE`; when enabled, `AcceptConnection` waits for service
	`ConnectionReady` before `OnReady` is issued.
- When `IRONRDP_WTS_CONTROL_PIPE` is not set, provider startup now probes the default
	pipe name (`IronRdpWtsControl`) and enables service polling only if `StartListen`
	handshake succeeds; otherwise it keeps bootstrap fallback behavior.
- Connection lifecycle state transitions and cleanup hooks are in place.
- Invalid connection method ordering now returns explicit transition errors.
- CredSSP policy gate (HYBRID/HYBRID_EX required) is wired in `AcceptConnection`.
- Side-by-side install and uninstall scripts are present under `scripts/`.
- Licensing, shadowing, virtual channels, and several advanced methods are still `E_NOTIMPL`.

## First-run preparation checklist

Run all commands from an elevated PowerShell session on a Windows test VM.

Prerequisite: host Remote Desktop must be enabled (`fDenyTSConnections = 0`).

Prerequisite: the host must expose the standard TCP RDP transport path (`TermDD` service key at
`HKLM\SYSTEM\CurrentControlSet\Services\TermDD`). Hosts that only expose non-TCP listeners
(for example `qwinsta` shows only an opaque listener name and no `RDP-Tcp`) cannot be used for
`mstsc` TCP side-by-side validation.

Optional preflight for Hyper-V fleets (run from host):

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\check-vm-side-by-side-eligibility.ps1 \
	-VmNames IT-HELP-TEST,IT-HELP-WAC,IT-HELP-DVLS \
	-AdminUser IT-HELP\Administrator \
	-AdminPasswordPlainText '<password>' \
	-PortNumber 4495
```

The script reports `Eligible = True/False` per VM and flags blockers such as missing `TermDD`.

### 1) Build the provider DLL

```powershell
cargo build -p ironrdp-wtsprotocol-provider --release
```

Expected artifact:

`target\release\ironrdp_wtsprotocol_provider.dll`

Optional helper script:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\build-provider-dll.ps1 -Profile release -Locked:$true
```

### Optional quick path (single orchestrator script)

Preview planned actions:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 -Mode Preview
```

Run full install flow:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 \
	-Mode Install \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-TargetHost <host> \
	-GenerateRdpFile \
	-PortNumber 4489
```

More robust first-run variant (restarts and waits for ready state):

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 \
	-Mode Install \
	-BuildProvider \
	-BuildProfile release \
	-TargetHost <host> \
	-PortNumber 4489 \
	-RestartTermService \
	-WaitForServiceReadyAfterRestart \
	-GenerateRdpFile
```

If `-ProviderDllPath` is omitted, the orchestrator auto-detects `target\<profile>\ironrdp_wtsprotocol_provider.dll`.
Add `-BuildProvider` to make it build automatically before install.
With `-RestartTermService -WaitForServiceReadyAfterRestart`, the orchestrator waits until `TermService` is running and the configured port is listening.

### Optional packaging path (prepare commands without installing)

Generate a ready-to-run package (install/rollback/connect command files + `.rdp` file) without touching system state:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\prepare-first-run-package.ps1 \
	-TargetHost <host> \
	-BuildProvider \
	-BuildProfile release \
	-PortNumber 4489
```

This creates `artifacts\first-run-package-<timestamp>` containing:

- `START-HERE.md`
- `run-first-run-elevated.ps1`
- `run-first-run.ps1`
- `run-first-run-elevated.cmd`
- `run-first-run.cmd`
- `run-preview.ps1`
- `run-preflight.ps1`
- `run-install.ps1`
- `run-install-restart.ps1`
- `run-verify.ps1`
- `run-smoke.ps1`
- `run-connect.ps1`
- `run-connect.cmd`
- `run-diagnostics.ps1`
- `run-diagnostics.cmd`
- `run-rollback.ps1`
- `run-rollback.cmd`
- `preview-now.ps1.txt`
- `preflight-now.ps1.txt`
- `first-run-now.ps1.txt`
- `install-now.ps1.txt`
- `install-with-restart-now.ps1.txt`
- `verify-now.ps1.txt`
- `smoke-now.ps1.txt`
- `rollback-now.ps1.txt`
- `connect-now.txt`
- `collect-diagnostics-now.ps1.txt`
- `manual-steps.ps1.txt`
- `irdp-side-by-side.rdp`
- `package.json`
- plus `artifacts\first-run-package-<timestamp>.zip`
- plus `artifacts\first-run-package-<timestamp>.zip.sha256.txt`
`-WaitForServiceReadyAfterRestart` requires `-RestartTermService`.

When `-PortNumber` is omitted, scripts resolve the listener port from
`HKLM\SOFTWARE\IronRDP\WtsProtocolProvider\ListenerPort` and fall back to `4489`.
To change the default for future runs:

```powershell
Set-ItemProperty -Path "HKLM:\SOFTWARE\IronRDP\WtsProtocolProvider" -Name "ListenerPort" -Type DWord -Value 4495
```

### 2) Backup current listener and provider state (recommended)

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\backup-side-by-side-state.ps1 \
	-ListenerName IRDP-Tcp
```

Keep the printed backup directory path for potential restore.
By default, backups are saved under `artifacts\wtsprotocol-backup-<timestamp>` and include `manifest.json`.

### 3) Install as a side-by-side listener

Optional but recommended preflight before install:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\preflight-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 4489
```

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\install-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 4489
```

What this script does:

- Clones `RDP-Tcp` listener configuration into `IRDP-Tcp` (if it does not exist yet).
- Registers protocol manager CLSID `{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}` under `HKLM\SOFTWARE\Classes\CLSID`.
- Sets `LoadableProtocol_Object` on `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp`.
- Sets `PortNumber` on the custom listener (default `4489`) so mstsc can target `<host>:4489`.
- Persists the resolved default listener port at `HKLM\SOFTWARE\IronRDP\WtsProtocolProvider\ListenerPort`.
- Fails fast if the selected port is already used by another WinStation listener.

Immediate registration verification:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\verify-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 4489
```

Optional consolidated smoke test before mstsc:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 4489
```

### 3) Restart Remote Desktop Services manually

Use your normal test-VM procedure, or pass `-RestartTermService` to the install script.

### 4) Configure firewall for custom listener port

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 \
	-Mode Add \
	-PortNumber 4489
```

By default, firewall rule name is derived from port: `IronRDP Side-by-side RDP (TCP <PortNumber>)`.

Optional verification:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 \
	-Mode Verify \
	-PortNumber 4489
```

### 5) Connect from mstsc

Connect to the test host with `mstsc` using `<host>:4489` and validate provider logs/method sequencing.

Example:

```powershell
mstsc /v:<host>:4489
```

Optional `.rdp` file generation:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\new-side-by-side-mstsc-file.ps1 \
	-TargetHost <host> \
	-PortNumber 4489 \
	-OutputPath .\artifacts\irdp-side-by-side.rdp
```

If you use the orchestrator with `-GenerateRdpFile`, this file can be generated automatically.

### Troubleshooting bundle (if mstsc fails)

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\collect-side-by-side-diagnostics.ps1 \
	-ListenerName IRDP-Tcp \
	-PortNumber 4489 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll
```

This writes a diagnostics folder under `artifacts\wtsprotocol-diagnostics-<timestamp>`.

### Input/graphics device handle overrides (if keyboard, mouse, or video do not initialize)

The provider now opens real handles in `GetInputHandles` and `GetVideoHandle`.
If your host uses non-default device names, set machine-level environment variables and restart `TermService`:

```powershell
[Environment]::SetEnvironmentVariable("IRONRDP_WTS_KEYBOARD_DEVICE", "\\.\KeyboardClass0", "Machine")
[Environment]::SetEnvironmentVariable("IRONRDP_WTS_MOUSE_DEVICE", "\\.\PointerClass0", "Machine")
[Environment]::SetEnvironmentVariable("IRONRDP_WTS_VIDEO_DEVICE", "\\.\RdpVideoMiniport", "Machine")
```

Then restart `TermService` and run verify/smoke again.

### Optional virtual-channel named-pipe bridge

To enable the default provider-side virtual-channel named-pipe bridge (recognized IronRDP VC endpoints only), set a machine environment variable and restart `TermService`:

```powershell
[Environment]::SetEnvironmentVariable("IRONRDP_WTS_VC_BRIDGE_PIPE_PREFIX", "IronRdpVcBridge", "Machine")
```

Pipe names are generated as:

- `\\.\pipe\<prefix>.svc.<channel>` for static channels
- `\\.\pipe\<prefix>.dvc.<channel>` for dynamic channels

Payload format is little-endian `u32` length prefix + raw bytes.

Directionality:

- provider → pipe (payload bytes read from TermSrv VC handles)
- pipe → provider (framed payload bytes written back into TermSrv VC handles)

### 6) Roll back

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\uninstall-side-by-side.ps1 -ListenerName IRDP-Tcp
```

Optional full restore from backup directory:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\restore-side-by-side-state.ps1 \
	-BackupDirectory <path returned by backup-side-by-side-state.ps1>
```

Optional firewall cleanup:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 -Mode Remove
```

Orchestrated rollback:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 -Mode Rollback -PortNumber 4489
```

Orchestrated rollback with automatic backup restore (latest backup if `-BackupDirectory` omitted):

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 \
	-Mode Rollback \
	-PortNumber 4489 \
	-RestoreBackupOnRollback
```

Then restart `TermService`.

## Bring-up flow summary

1. Build this crate as a Windows DLL (`cdylib`) in a controlled test environment.
2. Register the provider as a separate protocol entry using Microsoft’s custom-protocol provider registration workflow.
3. Restart Remote Desktop Services in the test environment.
4. Validate method-call sequencing and state transitions in logs.
5. Keep rollback path ready by disabling/removing the custom protocol registration.

## Notes

- Do not cut over the built-in protocol until the `E_NOTIMPL` paths are implemented.
- Keep plaintext credential fallback disabled; use CredSSP/SSPI/Kerberos-first flow.
- A companion service (e.g., `ceviche-rs`) is optional and not yet required for this in-proc provider scaffold.
