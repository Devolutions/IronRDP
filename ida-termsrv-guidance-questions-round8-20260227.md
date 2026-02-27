# IronRDP TermSrv Provider — IDA Guidance Questions (Round 8, 2026-02-27)

## Round-7 continuation actions completed
Using `artifacts/ida-termsrv-guidance-answers-round7-20260227.md`, we executed the recommended probes and captured strict + Event Viewer evidence.

### 1) Corrected `Execute` path test on WinStations key
- Enumerated WinStations keys on target VM:
  - `Console`
  - `IRDP-Tcp`
  - `RDP-Tcp`
- Applied A/B on:
  - `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp\Execute`
  - value set to `%SYSTEMROOT%\System32\userinit.exe`
- Strict run log:
  - `artifacts/ida-round7-step1-winstations-execute-userinit-run1.log`
- Observed (regression):
  - `Security4624Type10=0`
  - `NotifyCommandProcessCreated=0`
  - no screenshot produced
  - strict failure escalated to 8 issues
- Registry was restored after test:
  - `WinStations\IRDP-Tcp\Execute=''`

### 2) Baseline reconfirmation after restore
- Strict run log:
  - `artifacts/ida-round7-step2-baseline-correlation-run1.log`
- Baseline returned to prior known boundary:
  - Type10 + session-linked graphics proof: true
  - `NotifyCommandProcessCreated` present with
    - `userinit=false explorer=false logonui=true winlogon=true`
  - `RedirectMessage utype=16 ... "The data is invalid."`
  - still blank/uniform frame and Explorer=0

### 3) Event Viewer correlation artifact
- Captured to:
  - `artifacts/ida-round7-event-correlation-run1.log`
- Key signals captured:
  - LSM error: `ErrorCode 0xD0000001` during `Initialized -> EvCreated`
  - Application Error: `svchost.exe_TermService` fault in `termsrv.dll`
    - exception code `0xc0000005`
    - fault offset `0x000000000002d326`
  - repeated User Profile Service informational `Logon type: RDS`
- No direct Winlogon/GroupPolicy explicit error for `ERROR_INVALID_DATA` found in this sample.

### 4) Callback evidence checks from baseline run
- `IWRdsProtocolConnection::LogonNotify called`:
  - **not observed** in baseline strict log
- `WrdsLogonErrorRedirector::RedirectLogonError ... ntsstatus/ntssubstatus`:
  - **not observed** (only `RedirectStatus` and `RedirectMessage` observed)

---

## Updated branch map
- **Stable baseline branch (current best):**
  - Type10 and session graphics positive
  - command-process callback occurs pre-userinit (`userinit=false`)
  - `RedirectMessage("The data is invalid")`
  - no explorer
- **New hard-regression branch:**
  - setting `WinStations\IRDP-Tcp\Execute=userinit.exe` causes very early collapse (Type10=0 / no callback / no screenshot)
  - correlated with `termsrv.dll` AV crash signal in Event Viewer

---

## Round-8 questions for IDA reversing expert

### A. `Execute` override crash/collapse path (new)
1. Which exact code path consumes `WinStations\<listener>\Execute` in our `IRDP-Tcp` branch before session setup, and how can this path trigger an AV near `termsrv.dll+0x2d326`?
2. Is there an expected command format/quoting/tokenization contract for `Execute` in this branch (e.g., command line requiring args, forbidden env-expansion patterns) that `%SYSTEMROOT%\System32\userinit.exe` violates?
3. In `StartSession` and its callees, what null/length assumptions on command buffers can produce the observed LSM `ErrorCode 0xD0000001` + termsrv AV when Execute is present?
4. Is `IRDP-Tcp` routed through a listener type that expects a different initial command semantic than default RDP-Tcp?

### B. Why `LogonNotify` never appears in baseline
5. In this build, what precise event must occur for `CConnectionEx::OnLogonCompleted` to call provider `LogonNotify` (vtable+128), and which precondition is currently unsatisfied on our path?
6. Is `OnLogonCompleted` suppressed when `RedirectMessage(utype=16, "The data is invalid")` is raised, or should both occur on alternate branches?
7. Can you identify the shortest branch condition that differentiates:
   - path A: `NotifyCommandProcessCreated` only (our current)
   - path B: `OnLogonCompleted` + `LogonNotify` + userinit launch (expected)

### C. Redirect message origin narrowing
8. For the RPC path into `RpcRedirectLogonMessage`, what caller module/function in winlogon/session stack constructs the specific string "The data is invalid" on our branch?
9. Is there an adjacent path that calls `RpcRedirectLogonError` with NTSTATUS/substatus for the same failure class, and if yes, why is our run only seeing `RedirectMessage`?
10. What condition determines `utype=16` in this call path, and can that condition be used as a reliable branch discriminator for the failing stage?

### D. Minimal next A/B probes requested
11. Provide top 3 smallest probes with highest signal now, specifically to disambiguate:
   - execute-branch crash root cause,
   - logon-completed gate missing,
   - redirect-message source component.
12. For each probe, include one strict-harness disconfirming observable.

---

## Requested output format from reversing
For each key branch:
- function name + address
- condition expression
- expected inputs/state
- provider callback/property or registry value influencing that state
- expected external observable (strict gate / event log / provider log)
- one smallest disconfirming A/B change
