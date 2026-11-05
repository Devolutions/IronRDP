# TermService test VM

Use `IT-HELP-TERMSRV` for RDP and PSRemoting, with credentials from `RDP_USERNAME` and `RDP_PASSWORD`; never print or commit their values.
Use `ironrdp-agent` for RDP connections to this VM, not `mstsc` or MsRdpEx.
Sign IDD packages locally with `psign-tool` and the public Devolutions/psign test fixture, without trusting its CA on the build host.
Only the target VM trusts the public CA and publisher certificates; never copy the PFX/private key there.
See `..\ironrdp-idd\README.md` for setup and signing.
`scripts\deploy-testvm-psremoting.ps1 -WtsProvider` deploys only the companion, not the provider DLL.
To update the provider, rename the loaded `C:\IronRDPDeploy\provider\ironrdp_wtsprotocol_provider.dll` (TermService keeps it locked), copy the new DLL and PDB in, then rerun the deploy script to restart TermService; check the DLL's `LastWriteTime` on the VM.

## Debugging TermService with windbg-tool

Use [windbg-tool](https://github.com/Devolutions/windbg-tool) (`dotnet tool install -g Devolutions.WinDbg.Tool`) on dumps of `svchost.exe -k termsvcs` (TermService, which hosts the provider DLL), `ironrdp-termsrv.exe`, or a stalled session's `winlogon.exe` and `LogonUI.exe`.
Take dumps instead of using `live attach`: a debugger stop in TermService freezes every RDP session, and `target terminate` kills TermService, which has needed a VM reboot to restore native RDP.
The VM needs no .NET: copy `windbg-tool.exe` and its DLLs from the installed tool's `tools\net10.0\win-x64` folder under `%USERPROFILE%\.dotnet\tools\.store\devolutions.windbg.tool\` to `C:\IronRDPDeploy\windbg-tool\`.
`dump create --process-id <pid> --kind full --output <path>` is one-shot and needs no daemon; get TermService's PID from `(Get-CimInstance Win32_Service -Filter "Name='TermService'").ProcessId`.
`dump open` and `target thread` need the daemon: start it with `Start-Process windbg-tool.exe 'daemon','start'` without `-Wait`, because piping `daemon ensure` output or `Start-Process -Wait` blocks forever on the daemon's inherited handles.
Remove `_NT_SYMBOL_PATH` in the same command as every windbg-tool call, on the VM and on the build host: windbg-tool rejects `srv*` paths and downloads Microsoft symbols into its own `--symbol-cache`.
`dump triage` may load only exports for some stack modules, such as `dwmcore.dll`; subtract the module base from each frame, open a copy of the VM's DLL in IDA, which fetches the Microsoft PDB, and resolve the RVAs there.
Read call arguments spilled to the stack with `target memory`, using a frame's `stack_offset`.
For a hang, dump all threads with `target thread --engine-thread-id <n>` and look for TermService RPC threads in `RtlAcquireResourceShared`; `dump triage` only shows the current thread.
Run the analysis inside the VM over PSRemoting, or zip the dump and `Copy-Item -FromSession` it to the session artifacts directory and analyse it on the build host.
Dumps contain CredSSP and logon passwords: never commit them, and delete them from the VM once analysed.
Directory listings can report zero-length active logs; open the file with shared access and strip ANSI escapes before matching timestamps.

## Crashes and rejections

WER LocalDumps on the VM write crashes of TermService's svchost, `winlogon.exe`, `LogonUI.exe`, `dwm.exe`, and `WUDFHost.exe` to `C:\IronRDPDeploy\dumps\wer`.
Keep `dwm.exe` at `DumpType=1`: a full dump keeps the session's compositor frozen for about five minutes.
For duplicate RAIL handshakes, check Application Error 1000 and WER 1001 for `rdpshell.exe`; a restarted native shell can reopen RAIL without an RDP reconnect.
Set a temporary per-application `LocalDumps\rdpshell.exe` policy before reproducing; WER may delete the temporary dump while retaining only `Report.wer`.
TermService runs with strict handle checking, so closing a handle termsrv owns kills the service and the client sees a TLS EOF; termsrv owns the handles from `GetInputHandles` and `CreateVirtualChannel`.
When termsrv rejects or stalls a connection, trace its TraceLogging provider `{5283D5F6-65B5-425F-A30B-F16C057D6B57}` with `logman start <name> -ets -p '{5283D5F6-65B5-425F-A30B-F16C057D6B57}' 0xffffffffffffffff 0xff -o <etl>`; its events name the failing function and values.
`tracerpt -of XML` output runs to hundreds of megabytes, so filter it on the VM with a `StreamReader` and delete it afterwards.
