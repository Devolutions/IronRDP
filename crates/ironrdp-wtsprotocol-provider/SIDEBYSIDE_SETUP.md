# Side-by-side setup (first-run prep)

This crate currently provides an initial Windows protocol-provider scaffold using:

- `IWRdsProtocolManager`
- `IWRdsProtocolListener`
- `IWRdsProtocolConnection`

The intended rollout is side-by-side first (new protocol listener name), before any in-place replacement of the built-in protocol.

## Current status

- COM interface implementations are present.
- In-proc COM activation exports are present (`DllGetClassObject`, `DllCanUnloadNow`).
- Listener startup dispatches `OnConnected` from a dedicated worker thread.
- Connection lifecycle state transitions and cleanup hooks are in place.
- Invalid connection method ordering now returns explicit transition errors.
- CredSSP policy gate (HYBRID/HYBRID_EX required) is wired in `AcceptConnection`.
- Side-by-side install and uninstall scripts are present under `scripts/`.
- Licensing, shadowing, virtual channels, and several advanced methods are still `E_NOTIMPL`.

## First-run preparation checklist

Run all commands from an elevated PowerShell session on a Windows test VM.

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
	-PortNumber 3390
```

More robust first-run variant (restarts and waits for ready state):

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 \
	-Mode Install \
	-BuildProvider \
	-BuildProfile release \
	-TargetHost <host> \
	-PortNumber 3390 \
	-RestartTermService \
	-WaitForServiceReadyAfterRestart \
	-GenerateRdpFile
```

If `-ProviderDllPath` is omitted, the orchestrator auto-detects `target\<profile>\ironrdp_wtsprotocol_provider.dll`.
Add `-BuildProvider` to make it build automatically before install.
With `-RestartTermService -WaitForServiceReadyAfterRestart`, the orchestrator waits until `TermService` is running and the configured port is listening.

### 2) Backup current listener and provider state (recommended)

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\backup-side-by-side-state.ps1 \
	-ListenerName IRDP-Tcp
```

Keep the printed backup directory path for potential restore.

### 3) Install as a side-by-side listener

Optional but recommended preflight before install:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\preflight-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 3390
```

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\install-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 3390
```

What this script does:

- Clones `RDP-Tcp` listener configuration into `IRDP-Tcp` (if it does not exist yet).
- Registers protocol manager CLSID `{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}` under `HKLM\SOFTWARE\Classes\CLSID`.
- Sets `LoadableProtocol_Object` on `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp`.
- Sets `PortNumber` on the custom listener (default `3390`) so mstsc can target `<host>:3390`.
- Fails fast if the selected port is already used by another WinStation listener.

Immediate registration verification:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\verify-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 3390
```

Optional consolidated smoke test before mstsc:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-side-by-side.ps1 \
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
	-ListenerName IRDP-Tcp \
	-PortNumber 3390
```

### 3) Restart Remote Desktop Services manually

Use your normal test-VM procedure, or pass `-RestartTermService` to the install script.

### 4) Configure firewall for custom listener port

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 \
	-Mode Add \
	-PortNumber 3390
```

By default, firewall rule name is derived from port: `IronRDP Side-by-side RDP (TCP <PortNumber>)`.

Optional verification:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 \
	-Mode Verify \
	-PortNumber 3390
```

### 5) Connect from mstsc

Connect to the test host with `mstsc` using `<host>:3390` and validate provider logs/method sequencing.

Example:

```powershell
mstsc /v:<host>:3390
```

Optional `.rdp` file generation:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\new-side-by-side-mstsc-file.ps1 \
	-TargetHost <host> \
	-PortNumber 3390 \
	-OutputPath .\artifacts\irdp-side-by-side.rdp
```

If you use the orchestrator with `-GenerateRdpFile`, this file can be generated automatically.

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
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 -Mode Rollback -PortNumber 3390
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
