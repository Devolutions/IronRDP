# Side-by-side provider registration

These helpers register and recover the provider; they do not deploy the companion or display driver.
Start with the [companion deployment instructions] and [IDD setup instructions] for the test VM.

## Prerequisites

Run registration and recovery commands from the repository root in an elevated PowerShell on the test VM.
The provider requires Windows Server with the Remote Desktop Session Host (`RDS-RD-Server`) role, the matching companion, and the signed IDD.
Remote Desktop must be enabled (`fDenyTSConnections = 0`), with a standard `RDP-Tcp` listener and TCP transport (`HKLM\SYSTEM\CurrentControlSet\Services\TermDD`).
Keep the built-in listener on port 3389 and use a separate listener, such as `IRDP-Tcp` on port 4489, for the provider.

## Build and back up

```powershell
cargo build -p ironrdp-wtsprotocol-provider --release --locked
$modulePath = '.\server\termsrv\scripts\TestVm.psm1'
Import-Module $modulePath -Force
$targetDirectory = Get-CargoBuildOutputDirectory -WorkspaceRoot (Get-Location).Path
$providerDllPath = Join-Path $targetDirectory 'release\ironrdp_wtsprotocol_provider.dll'
```

Before registration, back up the listener and provider registry keys:

```powershell
$backupDirectory = .\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\backup-side-by-side-state.ps1 `
    -ListenerName IRDP-Tcp -PassThru
```

Record the backup directory path for recovery rather than relying on whichever backup is newest.
By default, backups are saved under `artifacts\wtsprotocol-backup-<timestamp>` and include `manifest.json`.

## Register and verify

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\install-side-by-side.ps1 `
    -ProviderDllPath $providerDllPath `
    -ListenerName IRDP-Tcp `
    -PortNumber 4489
```

The installer clones `RDP-Tcp` when needed, registers the [provider CLSID], and rejects ports already assigned to another WinStation listener.
Without `-PortNumber`, the helpers read `HKLM\SOFTWARE\IronRDP\WtsProtocolProvider\ListenerPort` and fall back to `4489`; the installer persists that resolved default.
Set the registry value to change the default for future invocations; an explicit `-PortNumber` overrides it only for that command.

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 `
    -Mode Add -PortNumber 4489
```

The firewall rule defaults to `IronRDP Side-by-side RDP (TCP <PortNumber>)`.
Use `-Mode Verify` to check it without changing it.

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\verify-side-by-side.ps1 `
    -ProviderDllPath $providerDllPath `
    -ListenerName IRDP-Tcp `
    -PortNumber 4489
```

This checks registration, not a live RDP session.
Start the companion before restarting TermService.
For PSRemoting deployments, run the [service restart helper] from the build host to preserve that ordering.

## Connect and diagnose

Set `IRONRDP_TEST_VM_HOSTNAME`, then connect from the build host with `ironrdp-agent`.
Supply `RDP_USERNAME` and `RDP_PASSWORD` through the environment.

```powershell
ironrdp-agent connect --server "$($env:IRONRDP_TEST_VM_HOSTNAME):4489" --no-prompt
ironrdp-agent status
```

Use the [isolated E2E harness][test-vm-tooling] for logon, session, and display proof.
To collect local provider state on the VM:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\collect-side-by-side-diagnostics.ps1 `
    -ListenerName IRDP-Tcp `
    -PortNumber 4489 `
    -ProviderDllPath $providerDllPath
```

This writes a diagnostics folder under `artifacts\wtsprotocol-diagnostics-<timestamp>`.
The [consolidated collector][test-vm-tooling] also gathers IDD and Type10 logon diagnostics over PSRemoting.

For input or display failures, inspect the provider log and [device handle notes].
Individual device-path environment overrides are not supported.
The legacy named-pipe bridge setting does not enable native virtual channels; see [virtual channel routing].

## Roll back

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\uninstall-side-by-side.ps1 -ListenerName IRDP-Tcp
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 `
    -Mode Remove -PortNumber 4489
```

To restore the saved registry keys as well, use the directory recorded before registration:

```powershell
.\server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts\restore-side-by-side-state.ps1 `
    -BackupDirectory $backupDirectory
```

Complete removal and any restore before restarting TermService.
If the backup restores provider registration, start the matching companion first.

## Notes

Licensing uses the RDS grace-period stub; shadowing and several advanced methods remain `E_NOTIMPL`.
See the [provider status] for implementation coverage; do not replace the built-in protocol until the `E_NOTIMPL` paths are implemented.
Keep plaintext credential fallback disabled; use CredSSP/SSPI/Kerberos-first flow.

[companion deployment instructions]: ../ironrdp-termsrv/README.md#deploy-to-a-test-vm-psremoting
[IDD setup instructions]: ../ironrdp-idd/README.md#set-up-a-test-vm
[provider CLSID]: ./README.md#protocol-manager-clsid
[service restart helper]: ../ironrdp-idd/scripts/restart-rds-services.ps1
[test-vm-tooling]: ../../README.md#test-vm
[device handle notes]: ./README.md#input-and-graphics-handle-notes
[virtual channel routing]: ./README.md#virtual-channel-routing-status
[provider status]: ./README.md#scope-current
