# Windows TermService Protocol Provider

Experimental Windows Remote Desktop Services (`TermService`) protocol provider and companion server built on IronRDP.

It registers a side-by-side WTS protocol listener so IronRDP can drive authenticated Windows sessions alongside native `RDP-Tcp` for differential testing and protocol development.

## Layout

- [`crates/ironrdp-termsrv`](crates/ironrdp-termsrv/README.md): Companion service and session-local capture, cursor, input, clipboard, and virtual-channel helpers.
- [`crates/ironrdp-wtsprotocol-provider`](crates/ironrdp-wtsprotocol-provider/README.md): In-process COM protocol provider (`IWRdsProtocolManager`, `IWRdsProtocolListener`, and `IWRdsProtocolConnection`) loaded by `TermService`.
- [`crates/ironrdp-wtsprotocol-ipc`](crates/ironrdp-wtsprotocol-ipc): Shared control-pipe and display-configuration messages between the provider, companion, and display driver.
- [`crates/ironrdp-idd`](crates/ironrdp-idd/README.md): UMDF Indirect Display Driver (IddCx) used for remote session display attachment and dynamic resizing.

## Test VM

Use Windows with PowerShell 5.1 or 7, a Rust MSVC toolchain, and a Windows test VM with PSRemoting enabled.
Set `IRONRDP_TEST_VM_HOSTNAME`, `RDP_USERNAME`, and `RDP_PASSWORD` for the target before running deployment commands from the repository root.
Never print or commit credential values.
The deployment account must be an administrator on the target.

```powershell
$env:IRONRDP_TEST_VM_HOSTNAME = 'rdp-test.example.com'
.\server\termsrv\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1 -WtsProvider -InstallIdd
```

The deployment stops TermService to replace the provider DLL, starts the companion before TermService, and checks that the companion owns the listener.
With `-InstallIdd`, it enables test signing and reboots the VM when needed.
The target needs the RDS Session Host role.
Installing the optional IDD also requires the Windows Driver Kit and `psign-tool` on the build host; see the [driver setup guide][driver-setup].
Set `IRONRDP_DEPLOY_ROOT` or pass `-RemoteRoot` to change the remote deployment directory.
Set `IRONRDP_IDD_REMOTE_PATH` or pass `-IddRemotePath` to change the remote IDD package directory.
Cargo output paths honor the `CARGO_TARGET_DIR` and `CARGO_BUILD_TARGET` environment variables.
The pinned public signing fixtures download on first use; pass `-TestCertificatesDirectory` to use a local directory containing `authenticode-test-ca.crt` and `authenticode-test-cert.pfx`.
Omit `-InstallIdd` when updating only the companion and provider.
The signing PFX and private key stay on the build host; only the target VM trusts the public test certificates.
With `-NoTermServiceStart`, the companion starts but you must start TermService yourself.
See [deployment options][deployment] for details and standalone mode.

Use `ironrdp-agent`, not `mstsc` or MsRdpEx, for RDP connections to the test VM.
Observe a connection through an isolated endpoint:

```powershell
.\server\termsrv\crates\ironrdp-termsrv\scripts\e2e-test-screenshot.ps1 -Mode Provider -SkipDeploy -StrictSessionProof
```

The E2E command uses the same hostname and credential environment variables.
It pins the target certificate only in a temporary local agent store and removes that store after the run.
The harness stops only its own client endpoint.
`-SkipDeploy` reuses installed binaries but still prepares IDD runtime state and a fresh graphics dump directory.
Session logoff/reset and Hyper-V power cycling require explicit opt-ins and exact targets.
Cleanup requires a desktop process with the requested account and domain; a short username from `quser` alone cannot authorize it.
Stale IDD devices are reported, not automatically removed.
Strict proof combines fresh client graphics with current-run logon, session, and IDD evidence; ambiguous or missing evidence fails.
The independent [session-local capture][capture] remains available to observe a connected desktop owned by the requested user.

Collect diagnostics without restarting services or changing registration:

```powershell
.\server\termsrv\scripts\collect-diagnostics.ps1
```

Its artifact directory contains provider data, an IDD archive, Type10 logon data, and `manifest.json`.
The manifest marks each section `ok`, `warning`, or `error`; partial collection preserves the successful sections.
Diagnostic files can contain usernames, addresses, and session data; review them before sharing.
The individual collectors and provider backup/restore commands remain independently callable.

## Helper tests

Run the local helper tests with Pester 5 or newer:

```powershell
Invoke-Pester -Path .\server\termsrv\tests
```

These tests do not deploy to or change the test VM.
They cover local orchestration and failure paths, not live driver loading, native service timing, or session capture.

## Debugging TermService with windbg-tool

Use [windbg-tool][windbg-tool] (`dotnet tool install -g Devolutions.WinDbg.Tool`) on dumps of `svchost.exe -k termsvcs` (TermService, which hosts the provider DLL), `ironrdp-termsrv.exe`, or a stalled session's `winlogon.exe` and `LogonUI.exe`.
Take dumps instead of using `live attach`: a debugger stop in TermService freezes every RDP session.
`target terminate` kills TermService; restoring native RDP may require a VM reboot.
Dumps can contain CredSSP and logon passwords: never commit them, and delete them from the VM once analysed.

The VM needs no .NET: copy `windbg-tool.exe` and its DLLs from the installed tool's `tools\net10.0\win-x64` folder under `%USERPROFILE%\.dotnet\tools\.store\devolutions.windbg.tool\` to `<deploy root>\windbg-tool\` (default: `C:\IronRDPDeploy\windbg-tool\`).
`dump create --process-id <pid> --kind full --output <path>` is one-shot and needs no daemon.
Get TermService's PID from `(Get-CimInstance Win32_Service -Filter "Name='TermService'").ProcessId`.
`dump open` and `target thread` need the daemon: start it with `Start-Process windbg-tool.exe 'daemon','start'` without `-Wait`, because piping `daemon ensure` output or using `Start-Process -Wait` can block on the daemon's inherited handles.
Remove `_NT_SYMBOL_PATH` in the same command as every windbg-tool call, on the VM and on the build host: windbg-tool rejects `srv*` paths and downloads Microsoft symbols into its own `--symbol-cache`.
Run the analysis inside the VM over PSRemoting, or zip the dump and use `Copy-Item -FromSession` to retrieve it into a local diagnostics directory outside the checkout.

For a hang, inspect all threads with `target thread --engine-thread-id <n>` and look for TermService RPC threads in `RtlAcquireResourceShared`; `dump triage` only shows the current thread.
Read call arguments spilled to the stack with `target memory`, using a frame's `stack_offset`.
`dump triage` may load only exports for some stack modules, such as `dwmcore.dll`.
Subtract the module base from each frame, open a copy of the VM's DLL in IDA, which fetches the Microsoft PDB, and resolve the RVAs there.
Directory listings can report zero-length active logs; open the file with shared access and strip ANSI escapes before matching timestamps.

## Crashes and rejected connections

When configuring WER LocalDumps on the VM, use `<deploy root>\dumps\wer` (default: `C:\IronRDPDeploy\dumps\wer`) for crashes of TermService's svchost, `winlogon.exe`, `LogonUI.exe`, `dwm.exe`, and `WUDFHost.exe`.
Keep `dwm.exe` at `DumpType=1`: full dumps can freeze the session's compositor for several minutes.
For duplicate RAIL handshakes, check Application Error 1000 and WER 1001 for `rdpshell.exe`; a restarted native shell can reopen RAIL without an RDP reconnect.
Set a temporary per-application `LocalDumps\rdpshell.exe` policy before reproducing; WER may delete the temporary dump while retaining only `Report.wer`.

TermService runs with strict handle checking, so closing a handle it owns kills the service and the client sees a TLS EOF.
TermService owns the handles returned by `GetInputHandles` and `CreateVirtualChannel`.
When TermService rejects or stalls a connection, trace its TraceLogging provider with `logman start <name> -ets -p '{5283D5F6-65B5-425F-A30B-F16C057D6B57}' 0xffffffffffffffff 0xff -o <etl>`; its events name the failing function and values.
`tracerpt -of XML` output can reach hundreds of megabytes, so filter it on the VM with a `StreamReader` and delete it afterwards.

[driver-setup]: crates/ironrdp-idd/README.md#set-up-a-test-vm
[deployment]: crates/ironrdp-termsrv/README.md#deploy-to-a-test-vm-psremoting
[capture]: crates/ironrdp-termsrv/scripts/capture-irdp-session-screenshot.ps1
[windbg-tool]: https://github.com/Devolutions/windbg-tool
