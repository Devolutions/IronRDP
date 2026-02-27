# IronRDP TermSrv Provider — IDA Guidance Questions (Round 3, 2026-02-27)

## Current Reassessment (post `E_NOTIMPL` hardening)

### Evidence used
- `artifacts/round3-consistency-output.log`
- `artifacts/consistency-phase1-run1.log`
- `artifacts/consistency-phase1-run2.log`
- `artifacts/consistency-phase1-run3.log`

### Stable, repeatable behavior (3/3 strict runs)
- Harness signature is identical across all 3 baseline runs:
  - `Exit=-1|Fail=2|4624=True|OnDriver=True|NotifyCmd=True|Reason17=False|Explorer=False|Shot=True|Bmp=True`
- Strict proof always reaches the same boundary:
  1. Type10 proof present
  2. provider/session-linked graphics proof present
  3. screenshot produced
  4. frame is blank/uniform
  5. `explorer.exe` absent in target session

### Current gate matrix
- `Security4624Type10`: **PASS** (consistent)
- `RemoteConnection261`: **PASS**
- `RemoteGraphics263`: **PASS**
- `NotifyCommandProcessCreated`: **PASS**
- Provider session proof markers: **PASS**
- Termsrv session proof markers: **PASS**
- Session-linked bitmap proof: **PASS**
- Screenshot produced: **PASS**
- Screenshot non-blank content: **FAIL**
- Explorer present in target session: **FAIL**

### Important detail (now verified)
For the four suspect property GUID paths, logs show all queried and all returned `E_NOTIMPL`:
- `PROPERTY_TYPE_CONNECTION_GUID` (`9EAA04F6-5B9D-4BA5-BE9D-3748AD6D8AF7`) -> `E_NOTIMPL`
- `PROPERTY_TYPE_SUPPRESS_LOGON_UI` (`846B20BB-6254-430E-952F-B0C7CA081915`) -> `E_NOTIMPL`
- `PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT` (`2918DB60-6CAE-42A8-9945-8128D7DD8E71`) -> `E_NOTIMPL`
- `PROPERTY_TYPE_LICENSE_GUID` (`4DAA5AB8-8B6A-49CF-9C85-8ADD504CD1F7`) -> `E_NOTIMPL`

This no longer causes the early-collapse behavior we saw when returning GUID values.

---

## Round-3 Questions for IDA Reversing Expert

### A. QueryProperty contract and hidden feature switches
1. In `termsrv.dll`, where are these 4 GUIDs consumed, and what exact branch behavior is taken when `QueryProperty` returns `E_NOTIMPL` vs `S_OK`?
2. Does returning `S_OK` for any of these implicitly switch TermService into an “official provider capability mode” (different state machine, expected callbacks, or stricter invariants)?
3. For each GUID, what is the expected `WTS_PROPERTY_VALUE.Type` and payload format in the successful path, and does TermService validate value stability across repeated queries?
4. Why is `PROPERTY_TYPE_CONNECTION_GUID` queried multiple times in the same connection? Is the value used as a key in a map/cache/session-object binding?
5. Is there any fallback default inside TermService equivalent to these properties when provider returns `E_NOTIMPL`?

### B. Shell transition boundary (current blocker)
6. Given callback sequence reaches `NotifyCommandProcessCreated`, which internal gate still decides whether `userinit.exe`/`explorer.exe` is launched in the target session?
7. Which function(s) in `termsrv.dll` or related components determine that session remains on `LogonUI/winlogon` instead of transitioning to Explorer?
8. Can you identify the exact branch/condition that keeps session in a pre-shell state while still allowing graphics/bitmap production?
9. Are there hidden dependencies on profile load success, GP completion, token integrity, or environment block completion after `NotifyCommandProcessCreated`?
10. Is there an expected provider callback after `NotifyCommandProcessCreated` that we currently stub/no-op and that can block final shell commit?

### C. Callback ordering and state machine expectations
11. Please recover expected ordering constraints between:
    - `AcceptConnection`
    - `GetClientData`
    - `GetUserCredentials`
    - `NotifySessionId`
    - `ConnectNotify`
    - `IsUserAllowedToLogon`
    - `NotifyCommandProcessCreated`
    - `LogonNotify`
12. Are any of these callbacks required to be idempotent but monotonic (first value wins, later calls ignored), and are we violating such assumptions?
13. Is session-id source precedence inside TermService documented in code (e.g., `NotifySessionId` vs `IsUserAllowedToLogon` vs `ConnectNotify`) and can mismatch cause shell suppression?

### D. Diagnostics to request from reverse engineering
14. Please provide function-level call graph + decision points from `OnConnected` through shell launch attempt, including HRESULT checks and boolean gates.
15. For each identified gate, provide:
    - condition expression
    - inputs/state variables
    - where each input is set
    - which provider callback influences it
16. If possible, map internal event/log strings (if any) to these gates so we can mirror high-signal instrumentation in provider logs.
17. Identify minimum provider-return set required for shell launch (strictly necessary) vs optional capability returns.

### E. Actionability request
18. Based on recovered logic, recommend the **smallest** provider behavior changes (if any) most likely to cross from current stable boundary (blank + no explorer) to explorer-launch state.
19. Rank top 3 hypotheses by confidence and expected implementation cost.
20. For each hypothesis, include one “disconfirming observation” we can test quickly in our strict harness.

---

## Notes for the reversing pass
- Current baseline is intentionally conservative for undocumented properties (`E_NOTIMPL`).
- We prefer preserving this conservative stance unless reversing evidence shows a strictly required return contract for shell progression.
- We are not currently chasing broad refactors—only root-cause transitions that affect shell launch boundary.
