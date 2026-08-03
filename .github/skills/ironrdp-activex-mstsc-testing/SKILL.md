---
name: ironrdp-activex-mstsc-testing
description: Launches the real mstsc.exe child through MsRdpEx mstscex.exe with ironrdpax.dll, process-local test credentials, standard ActiveX certificate configuration, UI Automation, and bounded native window-event coverage. Use when testing IronRDP ActiveX through MSTSC/MsRdpEx, debugging the native credential bridge, validating CLIPRDR startup, or monkey-testing the TscShellContainerClass host.
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
- For an authorized isolated test endpoint with a self-signed certificate, set the standard
  `IMsRdpClientAdvancedSettings4::AuthenticationLevel` property to `0` before `Connect`. This
  disables certificate and hostname validation for that control instance; never use it for a
  production connection.
- Do not synthesize messages with pointer-bearing parameters, especially `WM_DPICHANGED`. Generate
  native resize, move, activation, minimization, and restoration through Win32 window APIs instead.
- Store screenshots, traces, JSON reports, and temporary scripts under the session artifact
  directory, never in the repository.

## Prerequisites

1. Build the matched DLL:

   ```powershell
   cargo build -p ironrdp-activex --release
   ```

2. Locate an architecture-matched MsRdpEx installation containing `mstscex.exe` and `MsRdpEx.dll`.
   The [`scripts\Launch-NativeMstsc.ps1`](scripts/Launch-NativeMstsc.ps1) helper defaults to the
   standard installer location, `C:\Program Files\Devolutions\MsRdpEx`. Use `-MsRdpExDirectory`
   only when testing an explicit architecture-matched build elsewhere.
3. Confirm the user authorized the test endpoint, the process-local `RDP_USERNAME` /
   `RDP_PASSWORD` credentials, and (if required) `AuthenticationLevel=0`.

## Launch the real native host

Run [`scripts\Launch-NativeMstsc.ps1`](scripts/Launch-NativeMstsc.ps1) from the repository root.
It validates the MsRdpEx installation and DLL, sets the process-local IronRDP/MsRdpEx environment,
starts `mstscex.exe`, waits for its real `mstsc.exe` child, and returns both PIDs. Do not supply
`/axhost`, `/v:`, or an `.rdp` file: the native credential bridge must read the visible MSTSC
**Computer** field.

```powershell
$nativeHost = & .\.github\skills\ironrdp-activex-mstsc-testing\scripts\Launch-NativeMstsc.ps1 `
    -TracePath '<session-artifacts>\mstsc.trace'
$mstscPid = $nativeHost.MstscPid
```

The helper sets `IRONRDP_ACTIVEX_RPC=1`; use `ironrdp-agent --backend active-x` to drive its
listener. The target for UI Automation is the `mstsc.exe` child, not `mstscex.exe`.

Before calling `Connect`, configure the launched control through its normal preconnect COM
settings:

```text
IMsRdpClientAdvancedSettings4::put_AuthenticationLevel(0)
```

This is the only permitted invalid-certificate test configuration. It must be set explicitly for
the test control before connecting; do not use an environment variable, register the DLL, alter
machine-wide settings, or change the product default.

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
