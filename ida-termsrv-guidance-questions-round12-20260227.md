# IDA Termsrv/RDS Reversing Guidance — Round 12 Questions (2026-02-27)

Based on `artifacts/ida-termsrv-guidance-answers-round11-20260227.md`, we executed the recommended ETW-free probes and collected strict evidence.

## 1) Probe outcomes this round

### A. Probe 1 — minimal non-whitespace Execute trigger (`IRDP-Tcp\Execute='x'`)
- Log:
  - `artifacts/ida-round11-step1-irdptcp-execute-x-run1.log`
- Setup:
  - `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp\Execute = "x"`
- Outcome:
  - Early-collapse strict signature reproduced:
    - `Security4624Type10=0`
    - `NotifyCommandProcessCreated=0`
    - no provider/termsrv session proof markers
    - no screenshot produced
- Interpretation:
  - Any non-whitespace one-char command is sufficient to flip away from baseline null-command branch.

### B. Probe 2 — license GUID override A/B

#### B1) Temporary provider override to regular-desktop GUID
- Local temporary change (reverted afterward):
  - `PROPERTY_TYPE_LICENSE_GUID -> 0F0A4BF8-8362-435D-938C-222A518A8B78`

#### B2) `IRDP-Tcp\Execute='userinit.exe'` under GUID override
- Log:
  - `artifacts/ida-round11-step2-license-guid-regulardesktop-execute-userinit-run1.log`
- Outcome:
  - Still early-collapse strict signature (`Type10=0`, no callback marker, no screenshot).
  - In this failed path, `PROPERTY_TYPE_LICENSE_GUID` was not reached/logged before teardown.
- Interpretation:
  - This run does not support the hypothesis that license GUID alone rescues explicit-command path on IRDP-Tcp.

#### B3) Baseline reachability check under GUID override
- Log:
  - `artifacts/ida-round11-step2b-license-guid-regulardesktop-baseline-run1.log`
- Outcome:
  - Confirmed override was active in reachable baseline path:
    - `QueryProperty PROPERTY_TYPE_LICENSE_GUID -> 0F0A4BF8-8362-435D-938C-222A518A8B78`
  - Baseline remained same no-shell boundary (Type10+graphics+callback present; explorer absent).
- Interpretation:
  - Override plumbing worked, but did not alter the stable no-shell branch.

### C. Baseline restore and revalidation
- Reverted temporary code change locally and redeployed baseline binaries.
- Log:
  - `artifacts/ida-round11-step3-restore-baseline-build-run1.log`
- Outcome:
  - Confirmed baseline GUID restored:
    - `QueryProperty PROPERTY_TYPE_LICENSE_GUID -> 7D5E31F3-0FF8-4A25-9FCB-7B7E2F634001`
  - Baseline strict signature unchanged (2 issues: blank/uniform frame + explorer absent).

## 2) Consolidated state after round11 probes

- **Execute branch predicate:** now strongly supported that whitespace-only (`' '`) stays baseline/null-path while non-whitespace (`'x'`) flips to explicit-command failure path.
- **Crash branch relation:** `Execute='x'` reproduces early-collapse branch; in this run no fresh App Error 1000 appeared, but strict observables match the known pre-Type10 collapse class.
- **License-guid gate hypothesis:** setting regular-desktop license GUID was successfully applied in baseline path, yet explicit-command (`userinit.exe`) path still collapsed before useful progress.
- **No-shell baseline remains unchanged:** Type10 + session-linked graphics + callback occur, but `explorer.exe` never appears and screenshot remains uniform.

## 3) Focused round12 questions

### A. Execute parsing and start-session sub-branching
1. After non-whitespace `Execute` is accepted (e.g., `'x'`), what exact validation step decides between:
   - immediate pre-Type10 collapse,
   - later cleanup crash (`SmartPtr`/`CTSLicense`),
   - timeout-style failure?
2. Is there a command-shape gate (e.g., must contain path/extension) that routes `'x'` differently from `'userinit.exe'`?

### B. Explicit-command path internals on IRDP-Tcp
3. Which first failing function in IRDP-Tcp explicit-command path produces the observed no-Type10 teardown in our `'x'` run?
4. Where does this branch diverge from the known cleanup-AV branch (termsrv+0x2d326) captured earlier?

### C. License/connection identity influence scope
5. Since license GUID override was active in baseline path but explicit-command still collapsed, which other identity input dominates this branch?
   - `PROPERTY_TYPE_CONNECTION_GUID` (currently `E_NOTIMPL`)
   - listener metadata / handler flags
   - session object creation status
6. Is `PROPERTY_TYPE_LICENSE_GUID` consulted only after a gate we are not crossing in explicit-command IRDP-Tcp failures?

### D. No-shell branch persistence
7. In the stable baseline (Type10+graphics+callback), what concrete post-auth gate still blocks userinit/explorer launch?
8. Which one function/state transition should we instrument next (without ETW) to prove why `LogonNotify` is absent?

## 4) Requested next minimal probes (still ETW-resilient)

Please propose up to 3 highest-signal probes with:
- exact function/branch target,
- exact expected strict observable,
- one explicit disconfirming condition each,
- and no dependence on ETW files (disk is still constrained).

## 5) Artifacts (new this round)

- `artifacts/ida-round11-step1-irdptcp-execute-x-run1.log`
- `artifacts/ida-round11-step2-license-guid-regulardesktop-execute-userinit-run1.log`
- `artifacts/ida-round11-step2b-license-guid-regulardesktop-baseline-run1.log`
- `artifacts/ida-round11-step3-restore-baseline-build-run1.log`
