# IronRDP TermSrv Provider — IDA Guidance Questions (Round 7, 2026-02-27)

## Round-6 answer follow-up experiments completed
Using `artifacts/ida-termsrv-guidance-answers-round6-20260227.md`, we executed the top 2 registry A/B probes in isolation.

### A/B #1: `Execute` override on active WD key
- Target key discovered on test VM: `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\Wds\rdpwd`
- Baseline values before test:
  - `Execute=''`
  - `StartupPrograms='rdpclip'`
- Change applied:
  - `Execute='%SYSTEMROOT%\System32\userinit.exe'`
- Strict run log:
  - `artifacts/ida-round6-step1-execute-userinit-run1.log`
- Observed:
  - `OnConnected ... protocol_type=2`
  - `NotifyCommandProcessCreated ... userinit=false explorer=false logonui=true winlogon=true`
  - `Type10+graphics-in-target-session proof: True`
  - strict still fails with Explorer=0 and blank/uniform frame
- Result:
  - **Disconfirmed** (no boundary movement)

### A/B #2: `StartupPrograms` override on active WD key
- Change applied:
  - `StartupPrograms='%SYSTEMROOT%\System32\explorer.exe'`
- Strict run log:
  - `artifacts/ida-round6-step2-startupprograms-explorer-run1.log`
- Observed:
  - `NotifyCommandProcessCreated ... userinit=false explorer=false logonui=true winlogon=true`
  - `Type10+graphics-in-target-session proof: True`
  - strict still fails with Explorer=0 and blank/uniform frame
- Result:
  - **Disconfirmed** (no boundary movement)

### Consistent message path in both runs
- `WrdsLogonErrorRedirector::RedirectStatus` sequence remains:
  - “Please wait for the Group Policy Client”
  - “Please wait for the Local Session Manager”
  - “Welcome”
- Followed by:
  - `WrdsLogonErrorRedirector::RedirectMessage utype=16 ... message=The data is invalid.`

---

## Environment restoration after A/B
Registry values were restored to baseline on `rdpwd`:
- `Execute=''`
- `StartupPrograms='rdpclip'`

---

## Updated boundary (after round6 follow-up)
- Positive and stable:
  - Type10 logon
  - session-linked graphics proof
  - `NotifyCommandProcessCreated`
- Negative and stable:
  - callback-time `userinit=false`, `explorer=false`, `logonui=true`, `winlogon=true`
  - Explorer never appears in target session
  - frame remains blank/uniform
- Separate known collapse branch unchanged:
  - `CONNECTION_GUID -> S_OK` still aborts early (no Type10/session creation)

---

## Round-7 questions for IDA reversing expert

### A. Why did `Execute` / `StartupPrograms` A/B have no observable effect?
1. In `StartSession` (`0x18006d5f1`), what exact `WdName` string is used at runtime for our connection, and which concrete registry path is queried for `Execute`?
2. Is the queried value name exactly `Execute` in this build, or does the code branch to an alternate value/key for our provider/listener path?
3. Does `RegWinStationQueryValueW` read from a per-session virtualized hive or alternate control set rather than `HKLM\...\Wds\rdpwd`?
4. In the `v19==0`/empty-command path, does termsrv pass a null command intentionally to winlogon even when `Execute` is present (e.g., sanitized/rejected command)?

### B. ExecApps path viability
5. For `CExecSessionApps::CEventSink::ExecApps` (`0x180040310`), what precise event and gating conditions must be true for `StartupPrograms` to execute?
6. Is `ExecApps` skipped on our current path because a prerequisite state/event never occurs (e.g., post-logon state not reached)?
7. If `ExecApps` runs, what failure checks can prevent `CreateProcessAsUserW` from starting `explorer.exe` on `WinSta0\Default`?

### C. “The data is invalid” source path
8. Which caller supplies `utype=16` and message “The data is invalid” into `RedirectMessage` on this path?
9. What exact status/error code feeds this message (NTSTATUS/HRESULT/Win32), and from which component boundary (winlogon, userenv/profile, policy, LSM)?
10. Is that error a hard gate that prevents `userinit.exe` launch, or can userinit still launch after it on alternate branches?

### D. Command-process identity in callback
11. At `CConnectionEx::NotifyCommandProcessCreated` (`0x18007de30`), what object/process is treated as the “command process” in our failing branch where only winlogon/logonui are present?
12. What is the smallest state transition that would flip this callback-time snapshot from `userinit=false` to `userinit=true`?

---

## Requested output format from reversing
For each branch above, please include:
- function name + address
- condition expression
- runtime key/value or object being checked
- provider callback/property that can influence it (if any)
- expected external observable we can validate in strict harness
- one smallest disconfirming A/B probe to run next
