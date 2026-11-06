# Side-by-side provider registration

These helpers register the provider; they do not deploy the companion or display driver.
Start with the [companion deployment instructions] and [IDD setup instructions] for the test VM.

[companion deployment instructions]: ../ironrdp-termsrv/README.md#deploy-to-a-test-vm-psremoting
[IDD setup instructions]: ../ironrdp-idd/README.md#set-up-it-help-termsrv

The Windows protocol provider implements:

- `IWRdsProtocolManager`
- `IWRdsProtocolListener`
- `IWRdsProtocolConnection`

The intended rollout is side-by-side first (new protocol listener name), before any in-place replacement of the built-in protocol.

## Current status

See the [provider README](./README.md#scope-current) for implementation status.

- Side-by-side install and uninstall scripts are present under `scripts/`.
- Licensing uses the RDS grace-period stub.
- Shadowing and several advanced methods are still `E_NOTIMPL`.

## First-run preparation checklist

Run all commands from an elevated PowerShell session on a Windows test VM.
The provider requires Windows Server with the Remote Desktop Session Host (`RDS-RD-Server`) role, the matching companion, and the signed IDD.

Prerequisite: host Remote Desktop must be enabled (`fDenyTSConnections = 0`).

Prerequisite: the host must expose the standard TCP RDP transport path (`TermDD` service key at `HKLM\SYSTEM\CurrentControlSet\Services\TermDD`).
Hosts that expose only non-TCP listeners cannot be used for `mstsc` TCP side-by-side validation.
For example, an incompatible host might show only an opaque listener name and no `RDP-Tcp` in `qwinsta`.

Optional preflight for Hyper-V fleets (run from host):

```powershell
$password = ConvertTo-SecureString $env:RDP_PASSWORD -AsPlainText -Force
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\check-vm-side-by-side-eligibility.ps1 `
	-VmNames IT-HELP-TERMSRV `
	-AdminUser $env:RDP_USERNAME `
	-AdminPassword $password `
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
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\build-provider-dll.ps1 -Profile release -Locked:$true
```

### Optional quick path (single orchestrator script)

Preview planned actions:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 -Mode Preview
```

Run full install flow:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 `
	-Mode Install `
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll `
	-TargetHost 'IT-HELP-TERMSRV' `
	-GenerateRdpFile `
	-PortNumber 4489
```

More robust first-run variant (restarts and waits for ready state):

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 `
	-Mode Install `
	-BuildProvider `
	-BuildProfile release `
	-TargetHost 'IT-HELP-TERMSRV' `
	-PortNumber 4489 `
	-RestartTermService `
	-WaitForServiceReadyAfterRestart `
	-GenerateRdpFile
```

If `-ProviderDllPath` is omitted, the orchestrator auto-detects `target\<profile>\ironrdp_wtsprotocol_provider.dll`.
Add `-BuildProvider` to make it build automatically before install.
With `-RestartTermService -WaitForServiceReadyAfterRestart`, the orchestrator waits until `TermService` is running and the configured port is listening.

### Optional configuration package (prepare without installing)

Prepare a configuration and command guide locally without installing the provider or contacting the target:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\prepare-first-run-package.ps1 `
	-TargetHost 'IT-HELP-TERMSRV' `
	-BuildProvider `
	-BuildProfile release `
	-PortNumber 4489
```

Use `-ProviderDllPath` instead of `-BuildProvider` for an existing DLL.
Without either option, the generator uses `target\<profile>\ironrdp_wtsprotocol_provider.dll`.
It creates `artifacts\first-run-package-<timestamp>` with three files:

- `package.json`: the resolved checkout and DLL paths, endpoint, listener name, CLSID, and restart choice.
- `START-HERE.md`: commands that load the configuration and call the existing helpers directly.
- `irdp-side-by-side.rdp`: the endpoint for clients that consume `.rdp` files.

The default ZIP and `.zip.sha256.txt` checksum remain available; use `-CreateZip:$false` to skip them.
The folder and ZIP are not standalone deployment bundles: they contain no DLL, companion, IDD, or helper scripts.
The absolute checkout and DLL paths must exist on the server where the commands run; edit `package.json` if they move.
Keep `server\termsrv\scripts\TestVm.psm1` in that checkout; the installer imports it from the repository layout.
If you change the host or port, regenerate the `.rdp` file as well.
`TargetHost` sets the client endpoint; it does not redirect installation to a remote server.

On the server, open an elevated PowerShell in the package directory, review `package.json`, and follow `START-HERE.md`.
The guide keeps preflight, backup, install, offline registration verification, runtime smoke checks, diagnostics, uninstall, and restore separate.
Record the backup directory instead of relying on whichever backup is newest.
Use `ironrdp-agent` for connections to `IT-HELP-TERMSRV`.

The generator retains its parameters but no longer emits PS1/CMD launchers or duplicate command text files.
`-RestartTermService` records the choice used by the documented install command; it does not restart services during generation.
Runtime readiness checks are explicit commands, not an automatic forced restart.
Choose a new or empty `-OutputDirectory`; a nonempty directory is rejected rather than leaving stale launchers or deleting existing files.

When `-PortNumber` is omitted, scripts resolve the listener port from `HKLM\SOFTWARE\IronRDP\WtsProtocolProvider\ListenerPort` and fall back to `4489`.
To change the default for future runs:

```powershell
Set-ItemProperty -Path "HKLM:\SOFTWARE\IronRDP\WtsProtocolProvider" -Name "ListenerPort" -Type DWord -Value 4495
```

### 2) Backup current listener and provider state (recommended)

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\backup-side-by-side-state.ps1 `
	-ListenerName IRDP-Tcp
```

Keep the printed backup directory path for potential restore.
By default, backups are saved under `artifacts\wtsprotocol-backup-<timestamp>` and include `manifest.json`.

### 3) Install as a side-by-side listener

Optional but recommended preflight before install:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\preflight-side-by-side.ps1 `
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll `
	-ListenerName IRDP-Tcp `
	-PortNumber 4489
```

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\install-side-by-side.ps1 `
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll `
	-ListenerName IRDP-Tcp `
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
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\verify-side-by-side.ps1 `
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll `
	-ListenerName IRDP-Tcp `
	-PortNumber 4489
```

Optional consolidated smoke test before mstsc:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-side-by-side.ps1 `
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll `
	-ListenerName IRDP-Tcp `
	-PortNumber 4489
```

### 3) Restart Remote Desktop Services manually

Use your normal test-VM procedure, or pass `-RestartTermService` to the install script.

### 4) Configure firewall for custom listener port

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 `
	-Mode Add `
	-PortNumber 4489
```

By default, firewall rule name is derived from port: `IronRDP Side-by-side RDP (TCP <PortNumber>)`.

Optional verification:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 `
	-Mode Verify `
	-PortNumber 4489
```

### 5) Connect with ironrdp-agent

Connect to the test host on port 4489 and validate provider logs/method sequencing.
Supply `RDP_USERNAME` and `RDP_PASSWORD` through the environment.

Example:

```powershell
ironrdp-agent connect --server IT-HELP-TERMSRV:4489 --no-prompt
ironrdp-agent status
```

For clients that consume `.rdp` files, the optional generator remains available:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\new-side-by-side-mstsc-file.ps1 `
	-TargetHost 'IT-HELP-TERMSRV' `
	-PortNumber 4489 `
	-OutputPath .\artifacts\irdp-side-by-side.rdp
```

If you use the orchestrator with `-GenerateRdpFile`, this file can be generated automatically.

### Troubleshooting bundle

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\collect-side-by-side-diagnostics.ps1 `
	-ListenerName IRDP-Tcp `
	-PortNumber 4489 `
	-ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll
```

This writes a diagnostics folder under `artifacts\wtsprotocol-diagnostics-<timestamp>`.

For input or display failures, inspect the provider log and [device handle notes](./README.md#input-and-graphics-handle-notes).
Individual device-path environment overrides are not supported.
The legacy named-pipe bridge setting does not enable native virtual channels; see [virtual channel routing](./README.md#virtual-channel-routing-status).

### 6) Roll back

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\uninstall-side-by-side.ps1 -ListenerName IRDP-Tcp
```

Optional full restore from backup directory:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\restore-side-by-side-state.ps1 `
	-BackupDirectory 'C:\path\to\wtsprotocol-backup'
```

Optional firewall cleanup:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 -Mode Remove
```

Orchestrated rollback:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 -Mode Rollback -PortNumber 4489
```

Orchestrated rollback with automatic backup restore (latest backup if `-BackupDirectory` omitted):

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 `
	-Mode Rollback `
	-PortNumber 4489 `
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
- Provider registration alone does not provide an RDP listener; the matching `ironrdp-termsrv` companion must be running.
