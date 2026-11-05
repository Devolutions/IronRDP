# IronRDP TermSrv Windows Installer

WixSharp-based MSI packaging project for the `ironrdp-termsrv` Windows service.

This guide is the practical first-run flow for a Windows VM where you want to:

1. Install and run the `IronRdpTermSrv` Windows service.
2. Register the IronRDP WTS protocol provider side-by-side.
3. Connect with `mstsc` to the VM on the side-by-side listener port.

> The service is optional for the current in-proc provider scaffold, but this runbook includes it because many first-run environments want both pieces installed.

## What gets installed

- MSI product: `IronRDP TermSrv`
- Service name: `IronRdpTermSrv`
- Service executable name: `ironrdp-termsrv.exe`
- Install root (default): `%ProgramFiles%\IronRDP\TermSrv`

## Prerequisites (VM)

- Windows VM with Remote Desktop enabled.
- Elevated PowerShell session for install/register commands.
- VM firewall allows your chosen listener port (default `4489`).
- VM exposes standard TCP RDP transport (`HKLM\SYSTEM\CurrentControlSet\Services\TermDD` exists).
  Hosts that only expose non-TCP listeners cannot be validated with `mstsc /v:<host>:<port>`.

## 1) Build artifacts (host or VM)

From repository root:

```powershell
cargo build -p ironrdp-termsrv --release
cargo build -p ironrdp-wtsprotocol-provider --release
```

Expected files:

- `target\release\ironrdp-termsrv.exe`
- `target\release\ironrdp_wtsprotocol_provider.dll`

## 2) Build MSI

From `package\CevicheServiceWindowsManaged`:

```powershell
.\build-ceviche-service-msi.ps1 \
  -ServiceExePath ..\..\target\release\ironrdp-termsrv.exe \
  -ProviderDllPath ..\..\target\release\ironrdp_wtsprotocol_provider.dll \
  -Platform x64 \
  -Configuration Release
```

Expected MSI output:

- `package\CevicheServiceWindowsManaged\Release\IronRdpTermSrv.msi`

## 3) Install MSI in the VM

Copy `IronRdpTermSrv.msi` to the VM, then run:

```powershell
msiexec /i .\IronRdpTermSrv.msi /qn /norestart
```

Verify installation and service registration:

```powershell
Get-Service -Name IronRdpTermSrv
Get-ItemProperty "HKLM:\Software\IronRDP\TermSrv"
```

Start service (if not already running):

```powershell
Start-Service -Name IronRdpTermSrv
Get-Service -Name IronRdpTermSrv
```

## 4) Register side-by-side listener for mstsc

From repository root on the VM (elevated PowerShell):

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 \
  -Mode Install \
  -ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll \
  -TargetHost localhost \
  -RestartTermService \
  -WaitForServiceReadyAfterRestart \
  -GenerateRdpFile
```

Defaults:

- Listener name: `IRDP-Tcp`
- Port: `4489`

Port override key:

- `HKLM:\SOFTWARE\IronRDP\WtsProtocolProvider\ListenerPort`

Example override:

```powershell
Set-ItemProperty -Path "HKLM:\SOFTWARE\IronRDP\WtsProtocolProvider" -Name "ListenerPort" -Type DWord -Value 4495
```

## 5) Verify registration and firewall

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\verify-side-by-side.ps1 \
  -ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll

.\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 -Mode Verify
```

If firewall rule is missing:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\configure-side-by-side-firewall.ps1 -Mode Add
```

## 6) Connect with mstsc

From your client machine:

```powershell
mstsc /v:<vm-ip>:4489
```

Or use generated file:

- `artifacts\irdp-side-by-side.rdp`

## 7) Troubleshooting (recommended first commands)

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-side-by-side.ps1 \
  -ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll

.\crates\ironrdp-wtsprotocol-provider\scripts\collect-side-by-side-diagnostics.ps1 \
  -ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll
```

## 8) Rollback / uninstall

Unregister side-by-side listener:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 -Mode Rollback
```

Uninstall service MSI:

```powershell
msiexec /x .\IronRdpTermSrv.msi /qn /norestart
```

---

For deeper side-by-side details (backup/restore, manual preflight/install, diagnostics), see:

- `crates\ironrdp-wtsprotocol-provider\SIDEBYSIDE_SETUP.md`

## MSI build inputs (reference)

- `IRDP_TERMSRV_SERVICE_EXECUTABLE` (required): absolute path to service executable.
- `IRDP_PROVIDER_DLL` (optional): absolute path to `ironrdp_wtsprotocol_provider.dll`.
- `IRDP_TERMSRV_CONFIG_DIR` (optional): directory copied as `config\`.
- `IRDP_TERMSRV_MSI_VERSION` (optional): MSI version (`major.minor.build` constraints apply).
- `IRDP_TERMSRV_MSI_PLATFORM` (required in Release): `x64`, `x86`, or `arm64`.

Legacy env vars are also accepted for compatibility:

- `IRDP_CEVICHE_SERVICE_EXECUTABLE`
- `IRDP_CEVICHE_CONFIG_DIR`
- `IRDP_CEVICHE_MSI_VERSION`
- `IRDP_CEVICHE_MSI_PLATFORM`
