# IronRDP TermSrv Provider — IDA Guidance Questions (Round 5, 2026-02-27)

## Round-4 follow-up experiments completed
Based on `artifacts/ida-termsrv-guidance-answers-round4-20260227.md`, we executed the 3 recommended single-variable tests.

### Test matrix and outcomes

1) **Change 1: credential timing / `fInheritAutoLogon` readiness**
- Implementation: prefetch and cache credentials before `AcceptConnection` returns; `GetClientData` and `GetUserCredentials` consume cached credentials.
- Logs:
  - `artifacts/ida-round4-step1-credprefetch-run1.log`
  - `artifacts/ida-round4-step1-credprefetch-run2.log`
  - `artifacts/ida-round4-step1-credprefetch-run3.log`
- Observed:
  - `fetch_and_cache_connection_credentials cached ... source=accept_connection_prefetch`
  - `GetClientData filled_autologon_creds ...` in all runs
  - Type10/session/graphics/NotifyCommandProcessCreated remain positive
  - still `Explorer=0`, blank/uniform frame
- Result: **disconfirmed as sufficient** (timing fixed but shell boundary unchanged)

2) **Change 2: `PROPERTY_TYPE_CONNECTION_GUID` only (`S_OK + GUID`)**
- Implementation: only CONNECTION_GUID switched from `E_NOTIMPL` to `S_OK` stable GUID.
- Log:
  - `artifacts/ida-round4-step2-connectionguid-run1.log`
- Observed:
  - `PROPERTY_TYPE_CONNECTION_GUID -> <GUID>`
  - regression to early-collapse signature: `Security4624Type10=0`, `NotifyCommandProcessCreated=0`, no screenshot
- Result: **strong negative / destabilizing** in current environment

3) **Change 3: `PROPERTY_TYPE_LICENSE_GUID` only (`S_OK + GUID`)**
- Implementation: CONNECTION_GUID reverted to `E_NOTIMPL`; LICENSE_GUID switched to `S_OK + GUID`.
- Log:
  - `artifacts/ida-round4-step3-licenseguid-run1.log`
- Observed:
  - `PROPERTY_TYPE_LICENSE_GUID -> <GUID>`
  - CONNECTION_GUID remains `E_NOTIMPL`
  - Type10/session/graphics/NotifyCommandProcessCreated positive
  - still `Explorer=0`, blank/uniform frame
- Result: **no boundary improvement**

---

## Current best-known stable boundary
- Works reliably with:
  - `PROPERTY_TYPE_SUPPRESS_LOGON_UI -> 0`
  - `PROPERTY_TYPE_CONNECTION_GUID -> E_NOTIMPL`
  - `PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT -> E_NOTIMPL`
  - `PROPERTY_TYPE_LICENSE_GUID -> S_OK GUID` or `E_NOTIMPL` (both still no explorer)
  - credential prefetch/cache active
- Stable outcome:
  - Type10 + session-linked graphics + screenshot produced
  - `NotifyCommandProcessCreated=1`
  - still no Explorer (`Winlogon/LogonUI` desktop only), blank/uniform screenshot

---

## Round-5 questions for IDA reversing expert

### A. Why does CONNECTION_GUID `S_OK` collapse the session?
1. What exact code path consumes CONNECTION_GUID and causes the early-collapse signature when a GUID is returned?
2. Is there strict structural validation for returned GUID (format, relation to connection object, expected source) beyond type correctness?
3. Does termsrv expect CONNECTION_GUID to be correlated with another value (e.g., session object key, license GUID, reconnect key)?
4. Is there a hidden state transition triggered by `S_OK` for CONNECTION_GUID that requires additional provider behaviors we do not implement?
5. Which HRESULT/error branch causes the downstream loss of Type10 and NotifyCommandProcessCreated after CONNECTION_GUID is accepted?

### B. Post-NotifyCommandProcessCreated shell gating (still unsolved)
6. In the successful Type10 path where `NotifyCommandProcessCreated` is observed, what exact process is being reported (userinit, logonui, other)?
7. Where is the branch that should transition from command-process-created to explorer launch, and what condition fails on our path?
8. Is `LogonNotify` expected and/or required for explorer launch in this build? If yes, what prevents it?
9. Which winlogon/session-subsystem return code keeps desktop on LogonUI despite valid creds and LOGON_SUPPRESSION=0?

### C. Credential lifecycle semantics
10. Does `GetUserCredentials` need one-shot semantics (return once then empty), and can repeated successful returns block/alter shell progression?
11. Is `GetClientData` first-call content latched permanently, making later credential availability irrelevant?
12. If `GetClientData` is latched, which fields are latched and where in disassembly is that done?

### D. Protocol identity / mode effects
13. Does `ProtocolType=non-RDP` influence shell commit path even when Type10 succeeds?
14. Are there provider identity checks (CLSID/protocol name) that route custom providers away from the built-in shell-launch branch?

### E. Actionable next minimal tests requested from reversing
15. Please provide top 3 smallest provider-side A/B changes now most likely to flip `Explorer=0` to `Explorer=1`, with expected harness observables.
16. For each suggested test, include one disconfirming observable.

---

## Requested output format from reversing
For each key branch identified, include:
- function name / address
- condition expression
- required input values
- which provider callback/property sets those values
- expected external observable (event/log/harness gate)
