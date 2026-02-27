# IronRDP TermSrv Provider — IDA Guidance Questions (Round 9, 2026-02-27)

## Round-8 continuation experiments completed
Based on `artifacts/ida-termsrv-guidance-answers-round8-20260227.md`, we executed the highest-signal follow-up probes and captured strict/evidence artifacts.

---

## 1) Probe #1 (new local user) — executed and validated

### Account preparation
- Created local user on target VM:
  - `TestUser / TestPass123!`
- Added memberships:
  - `Users`
  - `Remote Desktop Users`

### First attempt (domain `IT-HELP`) — inconclusive formatting mismatch
- Log: `artifacts/ida-round8-step1-testuser-run1.log`
- Observed:
  - provider creds injected for `TestUser@IT-HELP`
  - `PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID ->` (empty)
  - strict: no matching Type10 for configured user
- Conclusion:
  - local-domain formatting likely wrong for SID/logon matching on this host.

### Corrected attempt (domain `IT-HELP-TEST`) — valid probe
- Log: `artifacts/ida-round8-step1b-testuser-netbios-run1.log`
- Observed:
  - `GetClientData`/`GetUserCredentials` for `TestUser@IT-HELP-TEST`
  - `PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID -> S-1-5-21-...-1001`
  - Type10+graphics proof: **True**
  - `NotifyCommandProcessCreated ... userinit=false explorer=false logonui=true winlogon=true`
  - strict still fails with same final shell boundary (2 issues: blank frame + explorer absent)
- Conclusion:
  - **disconfirms profile-corruption as user-specific root cause** (fresh local account still hits same no-shell boundary once probe is configured correctly).

---

## 2) Probe #3 (`Execute` command variants) — executed

All tests target:
- `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp\Execute`

### Variant A: `Execute=cmd.exe`
- Log: `artifacts/ida-round8-step3-execute-cmd-short-run1.log`
- Observed:
  - severe early collapse signature
  - `Security4624Type10=0`
  - `NotifyCommandProcessCreated=0`
  - no screenshot
  - strict fails with 8 issues

### Variant B: `Execute=C:\Windows\System32\cmd.exe`
- Log: `artifacts/ida-round8-step3-execute-cmd-fullpath-run1.log`
- Observed:
  - same severe early collapse signature as Variant A
  - strict fails with 8 issues

### Implication
- Regression is **not** specific to `%SYSTEMROOT%` expansion and **not** specific to `userinit.exe`.
- Any non-empty `Execute` tested so far appears to route into the same destabilizing branch for `IRDP-Tcp`.

---

## 3) Post-test restoration and baseline reconfirmation

- Restored:
  - `WinStations\IRDP-Tcp\Execute=''`
- Baseline reconfirmation log:
  - `artifacts/ida-round8-step4-postrestore-baseline-run1.log`
- Baseline behavior returned:
  - Type10+graphics proof true
  - callback-time process state still pre-userinit
  - blank frame + explorer missing (strict fails with 2 issues)

---

## 4) Correlation artifacts available

- Round7 event correlation (already captured):
  - `artifacts/ida-round7-event-correlation-run1.log`
  - includes LSM `ErrorCode 0xD0000001` and termsrv AV (`0xc0000005`) during Execute-override regression branch.

### Redirect path distinction observed in round8 runs
- `RedirectLogonError` was observed only in the misformatted-domain TestUser run:
  - `artifacts/ida-round8-step1-testuser-run1.log`
  - `ntsstatus=-1073741715`, `utype=16`, message "The user name or password is incorrect. Try again."
- In corrected TestUser run and baseline-equivalent runs, only `RedirectStatus`/`RedirectMessage` were observed (no `RedirectLogonError`).
- This supports the branch split in round8 answers: credential-auth failures use the NTSTATUS error path, while our persistent shell failure uses the message path after successful Type10.

---

## Updated branch model

### Stable baseline branch
- `Execute` empty
- Type10 + session graphics + callback-time marker present
- `userinit=false`, `explorer=false`
- `RedirectMessage("The data is invalid")`
- no shell transition

### Execute-override collapse branch
- `Execute` non-empty (`userinit`, `cmd`, full-path `cmd`) on `IRDP-Tcp`
- early collapse before Type10/session/callback
- strict 8-issue signature, no screenshot
- correlated with LSM/termsrv crash signals

---

## Round-9 questions for IDA reversing expert

### A. Non-empty Execute branch now appears command-agnostic
1. Which exact condition in `StartSession` / `ITSSession::Start` distinguishes `command == null` from `command != null`, and why does **any** non-empty command route to early teardown for `IRDP-Tcp`?
2. Is there a required object initialization sequence that occurs only in null-command mode and is skipped/broken in explicit-command mode?
3. Can you map the crash-correlated path from `StartSession` to the WIL destructor AV (`termsrv.dll+0x2d326`) with the immediate failing object type and owner?
4. Is there a listener-type gate (`IRDP-Tcp` vs `RDP-Tcp`) around explicit-command handling that leaves an internal pointer uninitialized?

### B. Fresh-user result disconfirms user-specific profile corruption
5. With a new local user achieving Type10 + graphics but still no shell, which shared (user-independent) winlogon/LSM condition still blocks `OnLogonCompleted`?
6. Does the branch that emits `RedirectMessage("The data is invalid")` depend on machine-wide state (policy/session object) rather than user profile integrity?

### C. Logon completion gate and callback sequence
7. Which exact signal from winlogon/LSM is missing before `CConnectionEx::OnLogonCompleted` can call provider `LogonNotify`?
8. Can `NotifyCommandProcessCreated` occur on a branch that is guaranteed never to reach `OnLogonCompleted`, and what state flag enforces that split?

### D. Timing discrepancy in shell evidence
9. In baseline runs, provider callback-time snapshot sees `logonui/winlogon=true`, but GUI snapshot later can show zero processes in target session. Which termsrv/session transitions explain this timing discrepancy?
10. Is the target session being recycled/renumbered between callback and later process snapshot in strict harness, or is this an expected desktop switch artifact?

### E. Next minimal probes requested
11. Please provide the top 3 smallest probes now to isolate:
   - explicit-command crash root cause,
   - missing `OnLogonCompleted` gate,
   - origin of `RedirectMessage("The data is invalid")` in shared machine state.
12. For each probe, include one disconfirming strict-harness observable.

---

## Requested output format from reversing
For each key branch:
- function name + address
- condition expression
- object/state required
- whether provider callback/registry can influence it
- strict-harness observable
- smallest disconfirming A/B probe
