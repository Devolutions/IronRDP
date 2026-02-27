# IronRDP TermSrv Provider — IDA Guidance Questions (Round 6, 2026-02-27)

## Round-5 continuation experiments completed
Based on `artifacts/ida-termsrv-guidance-answers-round5-20260227.md`, we executed the next two smallest A/B steps that were feasible provider-side.

### 1) `NotifyCommandProcessCreated` deep process-state instrumentation
- Change: added callback-time snapshot booleans for target session process presence:
  - `userinit`
  - `explorer`
  - `LogonUI`
  - `winlogon`
- Run log:
  - strict run output captured at `artifacts/remote-logs-20260227-125042/wts-provider-debug.key.log`
- Observed callback line:
  - `NotifyCommandProcessCreated called connection_id=1 session_id=3 userinit=false explorer=false logonui=true winlogon=true`
- Observed strict outcome:
  - Type10/session-linked graphics/screenshot: present
  - Explorer in target session: absent
  - screenshot: blank/uniform
- Interpretation:
  - We are still pre-`userinit`/shell transition at callback time.

### 2) `ProtocolType=1` (RDP) single-variable A/B
- Change: temporarily switched provider `ProtocolType` from non-RDP (2) to RDP (1), then ran strict once.
- Run logs:
  - `artifacts/remote-logs-20260227-125212/wts-provider-debug.log`
  - strict output with same run timestamp (`screenshot-20260227-125212.png`)
- Observed provider evidence:
  - `OnConnected ... protocol_type=1`
- Observed strict outcome:
  - still `NotifyCommandProcessCreated=1`
  - still `userinit=false explorer=false logonui=true winlogon=true`
  - still Explorer=0 and blank/uniform screenshot
- Result:
  - **Disconfirming for protocol-type as primary blocker**.

---

## Current best-known boundary (updated)
- Stable positive signals:
  - Type10 logon present
  - session-linked graphics proof present
  - `NotifyCommandProcessCreated` present
- Stable negative signals:
  - `userinit` not observed in target session at callback time
  - `explorer` not observed in target session
  - frame remains blank/uniform
- Strongly destabilizing branch:
  - `PROPERTY_TYPE_CONNECTION_GUID -> S_OK` still causes early collapse; baseline remains `E_NOTIMPL`.

---

## Round-6 questions for IDA reversing expert

### A. Winlogon -> userinit boundary (now primary)
1. In this build, where exactly is `NotifyCommandProcessCreated` emitted relative to the winlogon state machine (before/after any `CreateProcess(userinit.exe)` attempt)?
2. Which function/branch decides whether userinit is launched after credential acceptance in this provider path?
3. What concrete condition or return code keeps the session on `LogonUI + winlogon` without ever creating `userinit.exe`?
4. Is there a branch where `NotifyCommandProcessCreated` can fire for a non-userinit process while still remaining on the secure desktop? If yes, what process/object is bound to this callback?

### B. Message-path clue in logs
5. We repeatedly see `WrdsLogonErrorRedirector::RedirectMessage ... "The data is invalid."` shortly after logon progress and license-guid query. Which code path emits this message and what exact status/NTSTATUS/HRESULT maps to it?
6. Does that error path block userinit launch directly, or only affect status painting while launch can still proceed?

### C. Provider callback contract checks for userinit launch gate
7. Which provider callback result(s) are consumed in the specific branch that precedes userinit creation (e.g., `GetProtocolStatus`, `GetInputHandles`, `LogonNotify`, other), and what exact value checks are enforced?
8. Is there any hidden dependency on one-shot semantics for `GetUserCredentials` or an expected “credentials consumed” transition that, if not met, prevents winlogon from advancing to userinit?

### D. CONNECTION_GUID collapse branch (still unresolved)
9. Can you identify the precise reconnect/reattach branch entered when `CONNECTION_GUID` returns `S_OK`, including the failure check that causes teardown before Type10?
10. In that branch, is `AuthenticateClientToSession` failure (`E_NOTIMPL`) the direct abort trigger, or is there an earlier GUID/session correlation check that fails first?

---

## Requested output format from reversing
For each key branch above, please provide:
- function name + address
- condition expression
- required inputs/state
- provider callback/property feeding that input
- expected external observable (event/log/harness gate)
- one smallest disconfirming A/B probe we can run next
