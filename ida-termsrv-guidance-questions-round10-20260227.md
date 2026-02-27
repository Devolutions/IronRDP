# IronRDP TermSrv Provider — IDA Guidance Questions (Round 10, 2026-02-27)

## Round-9 continuation with WinDbg MCP (new)

This cycle used the new WinDbg MCP path with a real crash dump captured from the target VM, plus follow-up strict probes.

---

## 1) WinDbg MCP dump capture and analysis completed

### Dump capture workflow used
- Enabled remote LocalDumps for:
  - `HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\svchost.exe`
  - `HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\svchost.exe_TermService`
- Triggered known crash branch by setting:
  - `WinStations\IRDP-Tcp\Execute = cmd.exe`
- Captured remote dump:
  - `C:\CrashDumps\svchost.exe.380548.dmp`
- Copied to workspace:
  - `artifacts/windbg-dumps/svchost.exe.380548.dmp`

### WinDbg MCP command results (targeted)
Using MCP command:
- `.lastevent; .ecxr; r; kv; lmvm termsrv`
- `.exr -1; .ecxr; ub @rip L20; u @rip L20; dq @rcx L6; dq @rax L6; !error c0000005`

### Key debugger findings
- Exception:
  - `c0000005` access violation (read)
- Faulting instruction:
  - `termsrv!SmartPtr<IHealthStatus>::operator=+0x32`
  - address: `termsrv+0x2d326`
  - instruction: `mov rax, qword ptr [rax+10h]`
- Fault read address:
  - `0x00007ffab51685d8` (invalid/unmapped)
- Object context:
  - `rcx = 0x0000024f12535050` points to smart pointer holder
  - old pointee (`rdi`) resolves to `0x00007ffab51685c8` (non-null stale pointer)
- Stack (top relevant frames):
  - `termsrv!SmartPtr<IHealthStatus>::operator=`
  - `termsrv!CTSLicense::~CTSLicense`
  - `termsrv!CTSLicense::vector deleting destructor`
  - `termsrv!CTSLicense::CEventSink::Release`
  - `termsrv!CService::ServiceStopCleanupObjects`
  - `termsrv!CService::MiscThread`

### Updated interpretation from debugger evidence
This is now strongly a stale-pointer release path in license cleanup (`CTSLicense` / `IHealthStatus` smart-pointer assignment/release), not just a generic WIL destructor hypothesis.

---

## 2) Round-9 probe execution deltas (post-answer)

### Probe: fresh-user disambiguation (validated)
- Created and used local `TestUser` with corrected domain `IT-HELP-TEST`.
- Strict run:
  - `artifacts/ida-round8-step1b-testuser-netbios-run1.log`
- Observed:
  - SID resolution successful
  - Type10+graphics proof true
  - still `NotifyCommandProcessCreated ... userinit=false explorer=false logonui=true winlogon=true`
  - same no-shell boundary
- Meaning:
  - disconfirms user-specific profile corruption as primary cause.

### Probe: command-agnostic Execute crash (reconfirmed)
- `Execute=cmd.exe` and `Execute=C:\Windows\System32\cmd.exe` both collapse early.
- Logs:
  - `artifacts/ida-round8-step3-execute-cmd-short-run1.log`
  - `artifacts/ida-round8-step3-execute-cmd-fullpath-run1.log`
- Signature:
  - no Type10, no command-process callback, no screenshot, strict 8-issue failure.

### Post-test recovery
- Restored `WinStations\IRDP-Tcp\Execute=''`.
- Baseline returned (strict 2-issue boundary):
  - `artifacts/ida-round8-step4-postrestore-baseline-run1.log`

---

## Updated branch model (with debugger-backed crash branch)

### A) Baseline branch (stable)
- Type10 + session-linked graphics + callback marker
- callback-time `userinit=false`, `explorer=false`
- `RedirectMessage("The data is invalid")`
- no shell transition

### B) Execute-nonempty crash branch (debugger-backed)
- Any tested non-empty `Execute` on `IRDP-Tcp`
- early collapse + service failure
- dump shows AV in `termsrv!SmartPtr<IHealthStatus>::operator=` during `CTSLicense` cleanup path

---

## Round-10 questions for IDA reversing expert

### A. Crash branch (now symbolized by WinDbg)
1. In `CTSLicense` teardown paths, what exact control flow leads to `SmartPtr<IHealthStatus>::operator=` with a stale old pointer (`rdi`) under explicit-command (`Execute` non-empty) startup?
2. Which ownership contract is violated: double-release, use-after-free, or missing initialization before assignment in `CTSLicense`/`CEventSink` lifecycle?
3. What branch difference in `ITSSession::Start` / service cleanup is unique to `IRDP-Tcp + command!=null` that reaches `CService::ServiceStopCleanupObjects` with invalid license health object state?
4. Can you map where `IHealthStatus*` at smart-pointer slot (`rcx` object) is originally set, and where it is freed before this failing operator= call?

### B. Listener-type dependency
5. Is this `CTSLicense` stale-pointer crash reproducible for `RDP-Tcp` when `Execute` is non-empty, or specifically gated by `IRDP-Tcp` listener type/config objects?
6. Which listener/config field determines whether the explicit-command path initializes licensing health state correctly?

### C. Baseline no-shell branch (separate issue)
7. Given fresh-user success for Type10 and same failure afterward, what machine-wide state check still blocks `OnLogonCompleted` transition?
8. Where is `RedirectMessage("The data is invalid")` emitted in winlogon/LSM flow for this branch after successful Type10, and what exact upstream status causes it?

### D. Next minimal probes requested
9. Provide top 3 smallest probes now that best separate:
   - explicit-command crash root cause in `CTSLicense` lifetime,
   - listener-type gating (`IRDP-Tcp` vs `RDP-Tcp`),
   - machine-wide no-shell gate after Type10.
10. For each probe, include one strict-harness disconfirming observable.

---

## Requested output format from reversing
For each key branch:
- function name + address
- condition expression
- object ownership/state invariant
- whether provider callback/registry can influence it
- strict-harness + WinDbg observable
- smallest disconfirming A/B probe
