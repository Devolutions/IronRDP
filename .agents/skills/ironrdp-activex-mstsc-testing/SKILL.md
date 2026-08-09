---
name: ironrdp-activex-mstsc-testing
description: Test IronRDP ActiveX in the real mstsc.exe child launched by MsRdpEx, including the native credential bridge, shell behavior, resize handling, clipboard startup, and bounded UI stress. Use when validating or debugging ironrdpax.dll under native MSTSC. Do not use for MsRdpEx /axhost or generic COM smoke tests.
---

# Native MSTSC ActiveX testing

Exercise this exact host path:

```text
mstscex.exe -> mstsc.exe -> MsRdpEx.dll -> ironrdpax.dll
```

Read the native MSTSC, certificate, resize, and local RPC sections of `crates\ironrdp-activex\README.md` before testing.
Treat that README and `ironrdp-agent --help-agent` as the current behavior contract.

## Guardrails

- Use only an explicitly authorized endpoint and credentials.
- Load an architecture-matched `target\release\ironrdpax.dll` through `MSRDPEX_MSTSCAX_DLL`.
- Never register the test DLL, replace `mstscax.dll`, or alter machine-wide RDP settings.
- Keep credentials in the launcher process environment.
  Never print them or place them in commands, `.rdp` files, reports, traces, screenshots, or repository files.
- Keep scripts and artifacts in the session artifact directory.
- Do not synthesize pointer-bearing messages such as `WM_DPICHANGED`.
  Use UI Automation, `SendInput`, and Win32 window APIs to produce real UI and window events.
- Do not disable certificate validation by default.
  Follow the control's certificate-warning UI for an authorized endpoint.
  Set `AuthenticationLevel=0` only when the user explicitly authorizes that weaker policy for an isolated test endpoint and the chosen host exposes a supported preconnect setter.

## Launch and connect

1. Build with `cargo build -p ironrdp-activex --release`.
2. In one PowerShell process, set:

   ```powershell
   $env:MSRDPEX_MSTSCAX_DLL = (Resolve-Path .\target\release\ironrdpax.dll)
   $env:MSRDPEX_AX_BACKEND = 'ironrdp'
   $env:IRONRDP_ACTIVEX_NATIVE_MSTSC_CREDENTIAL_BRIDGE = '1'
   $env:IRONRDP_ACTIVEX_HOST_TRACE = '<session-artifacts>\mstsc.trace'
   ```

   Set `RDP_USERNAME` and `RDP_PASSWORD` only for CredUI prefill or authorized auto logon.
   Add `RDP_AUTOLOGON=1` only for an explicitly authorized unattended run.
   Add `IRONRDP_ACTIVEX_RPC=1` when post-connect inspection through `ironrdp-agent --backend active-x` is useful.
3. Snapshot existing `mstsc.exe` processes, then start the architecture-matched `mstscex.exe` without `/axhost`.
   Identify exactly one new `mstsc.exe` child by parent PID and creation time.
   Stop if pre-existing processes make the target ambiguous.
4. Target UI Automation at that child.
   Set the visible **Computer** field and invoke **Connect**.
   Approve CredUI for an interactive run, then handle any certificate warning according to the authorized test policy.
   Do not use the ActiveX RPC `connect` operation because it bypasses the native MSTSC bridge being tested.
5. Confirm the expected bridge route in the value-free trace:

   ```text
   NativeMstscCredentialBridge::StartProgramBridgeEnabled
   NativeMstscCredentialBridge::StartProgramBridgeAttached
   ```

   An interactive run must also reach `ActiveXCredentialPrompt::Prompt` and `ActiveXCredentialPrompt::CredentialsAccepted`.
   An unattended run must instead reach `NativeMstscCredentialBridge::AutoLogon`.
   Do not require username-form, certificate, clipboard, resize, or redraw markers when the corresponding feature or condition was not exercised.

## Exercise and assess

Wait for a connected state and a rendered frame before stress testing.
Use `ironrdp-agent --backend active-x status` and `screenshot` when RPC is enabled.

Apply a bounded sequence of moves, valid resizes, minimize/restore, maximize/restore, hide/show, activation, invalidation, safe pointer input, wheel input, and a harmless key press such as `VK_F24`.
Exercise SmartSizing or Zoom only when the native UI exposes it.
Space resize changes by at least 250 ms.

After every case, require:

- The selected `mstsc.exe` PID is alive.
- `IsWindow` is true and `IsHungAppWindow` is false.
- `SendMessageTimeout(WM_NULL, SMTO_ABORTIFHUNG, 500 ms)` succeeds.
- `GetWindowRect` reports positive dimensions after restoring the window.
- The session remains connected unless the case intentionally disconnects it.

For resize coverage, accept an in-session display update or a bounded reconnect fallback.
A `RdpWorker::DisplayResizeFallback:*` marker is diagnostic, not a failure, unless the session stays disconnected or the host becomes unresponsive.
Fail on a connection failure, fatal event, unexpected disconnect, process exit, hung window, or missing rendered frame.

## Teardown and report

Close the native shell through its UI and accept the standard disconnect confirmation.
Wait for the selected child and launcher PIDs to exit.
If normal teardown fails, stop only those known PIDs.
Require `IConnectionPoint::Unadvise` in the trace, clear the process-local environment variables, and report only aggregate case results, liveness, value-free markers, and non-sensitive artifact paths.
