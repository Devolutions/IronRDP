---
name: ironrdp-activex-mstsc-testing
description: Launches the real mstsc.exe child through MsRdpEx mstscex.exe with ironrdpax.dll, process-local test credentials, certificate bypass, UI Automation, and bounded native window-event coverage. Use when testing IronRDP ActiveX through MSTSC/MsRdpEx, debugging the native credential bridge, validating CLIPRDR startup, or monkey-testing the TscShellContainerClass host.
---

# IronRDP ActiveX native MSTSC testing

Use this workflow only for an explicitly authorized test endpoint and credentials. It validates the
actual path:

```text
mstscex.exe -> mstsc.exe -> MsRdpEx.dll -> ironrdpax.dll
```

`mstscex.exe /axhost` is **not** this workflow: it uses MsRdpEx's standalone ActiveX host rather
than launching the real `mstsc.exe` child.

## Safety boundary

- Build and load `target\release\ironrdpax.dll` explicitly through
  `MSRDPEX_MSTSCAX_DLL`; never register it globally or replace `mstscax.dll`.
- Require `MSRDPEX_AX_BACKEND=ironrdp` so MsRdpEx enables its private-layout exclusion.
- Put `RDP_USERNAME` and `RDP_PASSWORD` only in the launcher process environment. Never print,
  persist, pass as arguments, add to an `.rdp` file, or copy to artifacts.
- `IRONRDP_ACTIVEX_DANGEROUS_ACCEPT_INVALID_CERTIFICATE=1` bypasses certificate validation. Use it
  only for an authorized test run, set it on that process only, and never make it a product default.
- Do not synthesize messages with pointer-bearing parameters, especially `WM_DPICHANGED`. Generate
  native resize, move, activation, minimization, and restoration through Win32 window APIs instead.
- Store screenshots, traces, JSON reports, and temporary scripts under the session artifact
  directory, never in the repository.

## Prerequisites

1. Build the matched DLL:

   ```powershell
   cargo build -p ironrdp-activex --release
   ```

2. Obtain an architecture-matched MsRdpEx build containing `mstscex.exe` and `MsRdpEx.dll`.
3. Confirm the user authorized the test endpoint, the process-local `RDP_USERNAME` /
   `RDP_PASSWORD` credentials, and (if required) certificate bypass.

## Launch the real native host

Set the variables in a single PowerShell process before starting `mstscex.exe`. Do not supply
`/axhost`, `/v:`, or an `.rdp` file: the native credential bridge must read the visible MSTSC
**Computer** field.

```powershell
$env:MSRDPEX_MSTSCAX_DLL = (Resolve-Path .\target\release\ironrdpax.dll)
$env:MSRDPEX_AX_BACKEND = 'ironrdp'
$env:IRONRDP_ACTIVEX_NATIVE_MSTSC_CREDENTIAL_BRIDGE = '1'
$env:IRONRDP_ACTIVEX_DANGEROUS_ACCEPT_INVALID_CERTIFICATE = '1' # authorized test only
$env:IRONRDP_ACTIVEX_HOST_TRACE = '<session-artifacts>\mstsc.trace'

$launcher = '<MsRdpEx-build>\Release\mstscex.exe'
$launcherProcess = Start-Process -FilePath $launcher -PassThru
Start-Sleep -Seconds 7
$mstsc = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($launcherProcess.Id)" |
    Where-Object Name -ieq 'mstsc.exe' |
    Select-Object -First 1
if ($null -eq $mstsc) { throw 'mstscex did not launch mstsc.exe' }
```

The target for UI Automation is the `mstsc.exe` child, not `mstscex.exe`.

## Connect without exposing credentials

Use UI Automation to set the visible **Computer** edit control to the authorized endpoint and invoke
**Connect**. The explicit bridge will receive native MSTSC's `put_StartProgram` preflight, display
the IronRDP CredUI dialog, and pre-populate it from the process-local `RDP_USERNAME` /
`RDP_PASSWORD` values. Invoke **OK** only after confirming the prompt belongs to the launched
`mstsc.exe` process.

Expected value-free trace markers, in order:

```text
NativeMstscCredentialBridge::StartProgramBridgeEnabled
NativeMstscCredentialBridge::StartProgramBridgeAttached
ActiveXCredentialPrompt::Prompt
NativeMstscCredentialBridge::QualifiedUsername
ActiveXCredentialPrompt::CredentialsAccepted
RdpWorker::TlsCertificateValidation:DangerouslyAcceptInvalidCertificate
ActiveXClipboard::Started
Renderer::NativeMstscShellLayoutSynced
RdpWorker::PostLogonDisplayRedraw
```

Fail the run if the child cannot be found, the trace contains `ConnectionFailure`, `Fatal`, or an
unexpected `Disconnected` marker before test teardown.

## Native event matrix

After connection, rediscover the `TscShellContainerClass` top-level window. For every case, assert:

- `mstsc.exe` is alive.
- `IsWindow` is true.
- `IsHungAppWindow` is false.
- `SendMessageTimeout(WM_NULL, SMTO_ABORTIFHUNG, 500 ms)` returns.
- `GetWindowRect` yields positive width and height.

Use real APIs and input delivery to cover:

1. Move and resize through a bounded range of valid dimensions.
2. Minimize/restore and maximize/restore.
3. Hide/show and foreground/focus changes.
4. Invalidate/redraw the host to exercise paint/layout.
5. Real pointer movement, click, and wheel at a safe client coordinate.
6. A harmless real key press/release, such as `VK_F24`.
7. Native system-menu SmartSizing and Zoom changes, where exposed.
8. Slow, spaced resize changes (at least 250 ms apart) to observe Display Control or its bounded
   reconnect fallback.

The existing control debounces shell layout changes. `Renderer::DisplayResizeRequested` plus
`RdpWorker::PostLogonDisplayRedraw` is valid. A bounded
`RdpWorker::DisplayResizeFallback:ReactivationTimedOut` is diagnostic evidence of a server without
Display Control; it must not leave the session disconnected or the host unresponsive.

## Teardown

Close the native shell through its UI, accept its standard disconnect confirmation, then wait for
both `mstsc.exe` and `mstscex.exe` to exit. The trace must contain
`IConnectionPoint::Unadvise`. If the normal close path is blocked, stop only the known child and
launcher PIDs; never kill by process name.

Record only aggregate case counts, process/window liveness, static trace markers, and non-sensitive
screenshots in the final report.
