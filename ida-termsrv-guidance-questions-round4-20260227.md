# IronRDP TermSrv Provider — IDA Guidance Questions (Round 4, 2026-02-27)

## Why this round
Round 3 recommended a minimal change: return `S_OK` with `ULONG(0)` for `PROPERTY_TYPE_SUPPRESS_LOGON_UI` while keeping the other three suspect QueryProperty GUIDs as `E_NOTIMPL`.

We implemented exactly that and re-ran strict validation 3 times.

## New empirical result (post-change)
Using:
- `artifacts/ida-round3-step1-logonsuppress-run1.log`
- `artifacts/ida-round3-step1-logonsuppress-run2.log`
- `artifacts/ida-round3-step1-logonsuppress-run3.log`

Observed in all 3 runs:
- `PROPERTY_TYPE_SUPPRESS_LOGON_UI -> 0` (queried and returned as expected)
- `Security4624Type10`: present
- `NotifyCommandProcessCreated`: present
- session-linked graphics proof: present
- screenshot produced: yes
- screenshot content: blank/uniform
- `explorer.exe` in target session: absent

So, changing LOGON_SUPPRESSION alone does **not** cross the shell boundary in this environment.

---

## Gate status reassessment (current)

Stable passing gates:
- Type10 proof
- provider session-binding markers
- termsrv capture-session apply markers
- NotifyCommandProcessCreated
- session-linked bitmap proof

Stable failing boundary:
- target session remains `Winlogon/LogonUI` desktop (`Explorer=0`, `GuiProcesses=1`)
- frame is blank/uniform despite ongoing graphics updates

Interpretation:
- We are now past the previous “property-mismatch collapse” risk.
- The blocker is likely in a deeper winlogon/session-subsystem branch, not in the simple LOGON_SUPPRESSION value.

---

## Round-4 questions for IDA reversing expert

### 1) Precise consumer path for LOGON_SUPPRESSION
1. In current Windows build, where is `PROPERTY_TYPE_SUPPRESS_LOGON_UI` consumed after `QueryProperty` returns `S_OK`?
2. What exact boolean or enum does `ULONG(0)` set internally, and what branch should that drive?
3. Is that value later overwritten by policy/session flags (e.g., listener settings, user flags, reconnect state)?
4. Are there additional preconditions for suppressing logon UI besides this property (e.g., valid autologon blob presence, credential source flags, session kind)?

### 2) Credential acceptance vs shell launch
5. Which function confirms that credentials from `GetClientData` / `GetUserCredentials` are accepted for interactive shell logon?
6. Is there a branch where credentials are accepted enough to produce Type10 but still considered “non-shell” or “pre-shell only”?
7. Does winlogon require any additional flag from termsrv/provider to proceed from command-process-created to userinit/explorer?
8. Please identify where `NotifyCommandProcessCreated` sits relative to the internal “launch shell” decision (before/after/forked path).

### 3) Session class / desktop mode mismatch hypothesis
9. Can a session be classified as remote-interactive for auditing (Type10) yet remain in a restricted desktop mode that never launches explorer?
10. Which internal session attributes (token type, policy bits, session role, protocol type) force LogonUI persistence?
11. Does reporting `ProtocolType=2` (non-RDP in our provider schema) alter downstream shell expectations?
12. Are there checks tied to official provider CLSID/protocol identity that affect shell commit even when callbacks succeed?

### 4) QueryProperty set completeness hypothesis
13. After LOGON_SUPPRESSION is fixed, do any of the other three currently-`E_NOTIMPL` GUIDs become mandatory in this code path?
14. If yes, for each required property:
    - expected `Type`
    - expected value semantics
    - whether value must be stable across repeated calls
15. Specifically, can `PROPERTY_TYPE_CONNECTION_GUID` or `PROPERTY_TYPE_LICENSE_GUID` be required for the final userinit path (not just licensing/reconnect bookkeeping)?

### 5) Winlogon/userinit branch tracing request
16. Please provide the concrete call chain from successful RDS logon decision to `CreateProcess(userinit.exe)`.
17. On our failing path, where does execution diverge from the successful path?
18. For that divergence point, list all inputs and their sources (provider callback outputs, registry, policy, token fields).
19. Is there an internal timeout/race that can leave session on LogonUI while still emitting graphics and command-created callbacks?

### 6) Registry/policy interplay (code-level, not environment guesswork)
20. Which registry/policy checks are read in the failing branch before userinit launch?
21. Which of those are hard blockers vs optional fallbacks?
22. Is there any gate involving `Userinit`, `Shell`, `AutoAdminLogon`, or RDS-specific policy that can be inferred directly from disassembly conditions?

### 7) Actionable next experiments requested from reversing
23. Recommend the top 3 **single-variable** provider-side changes to test next, ranked by confidence.
24. For each change, provide an explicit expected observable in our strict harness (what should flip from 0→1).
25. Provide one disconfirming observable for each change.

---

## Constraints for next implementation cycle
- Keep `PROPERTY_TYPE_CONNECTION_GUID`, `PROPERTY_TYPE_LICENSE_GUID`, `PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT` conservative unless reversing evidence says otherwise.
- Avoid broad refactors; prefer one-variable A/B tests.
- Keep strict harness as ground truth; evaluate by gate flips, not subjective UI behavior.

---

## Appendix: current test signatures
All three post-change runs share this boundary signature shape:
- `Type10=1`
- `NotifyCommandProcessCreated=1`
- `Session-linked graphics proof=True`
- `Explorer=0`
- `Result=blank/uniform screenshot`
