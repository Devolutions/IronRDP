# IDA Termsrv/RDS Reversing Guidance — Round 13 Questions (2026-02-27)

Disk-space blocker is resolved and we completed the previously blocked ETW/state-correlation probe from round10/11 guidance.

## 1) What we executed

### A. ETW trace enablement (post-cleanup)
- VM free space was recovered and trace creation now works.
- First minimal LSM-only attempt produced ETL header only (no payload rows):
  - `artifacts/LSMTraceRound12-20260227-163709.etl`
  - `artifacts/LSMTraceRound12-20260227-163709.csv`

### B. Improved TS state trace capture during strict baseline
- Started collector with both providers and full keyword mask:
  - `Microsoft-Windows-TerminalServices-LocalSessionManager`
  - `Microsoft-Windows-TerminalServices-RemoteConnectionManager`
- Ran strict baseline while trace was active.
- Exported artifacts:
  - `artifacts/TSStateTraceRound12-20260227-163852.etl`
  - `artifacts/TSStateTraceRound12-20260227-163852.csv`
  - strict run log: `artifacts/ida-round12-step2-tsstate-etw-baseline-run1.log`

### C. Human-readable channel correlation for same window
- Pulled operational logs to avoid relying only on raw ETW numeric rows:
  - `artifacts/ida-round12-step3-lsm-rcm-operational-window.log`

## 2) Observed outcomes

### A. Strict baseline result unchanged
From `ida-round12-step2-tsstate-etw-baseline-run1.log`:
- Type10 + session-linked graphics proof: true
- callback present
- still no explorer launch, blank/uniform screenshot
- same 2-issue no-shell boundary

### B. ETW payload now present
From `TSStateTraceRound12-20260227-163852.csv`:
- TerminalServices provider rows captured (non-header):
  - RemoteConnectionManager: 7509 rows
  - LocalSessionManager: 15 rows
- Top event IDs in this capture:
  - `20502` dominant
  - plus `38`, `20523`, `2305`, `258`, `1143`, `261`, `41`, `20524`, `263`, `1158`, `272`, `40`, `58`

### C. Decoded operational transitions (same run window)
From `ida-round12-step3-lsm-rcm-operational-window.log`:
- LSM:
  - Event 41: Begin session arbitration (sessions 5 then 4)
  - Event 40: Session disconnected, reason code 0 (for those sessions)
- RCM:
  - Listener connection events for IRDP-Tcp
  - Listener terminal-class announcements (`20523`)
  - WDDM enabled (`263`)
  - policy/credential informational events (`1143`, `20524`)

## 3) Interpretation to validate

- We now have evidence that session arbitration starts, but sessions still disconnect with reason 0 before shell transition.
- No explicit high-severity decoded failure event appears in this narrow window; this may indicate failure in a lower-level state edge not surfaced as Error-level operational events.
- The no-shell branch remains stable despite ETW visibility now being restored.

## 4) Focused round13 questions

### A. Decode-level mapping from ETW IDs to internal states
1. In this build, what do LSM/RCM event IDs `40/41/38/58/2305/20502` map to in termsrv/LSM internal state-machine edges?
2. Which of those IDs (or sequences) correspond to "post-auth success but pre-userinit disconnect"?

### B. Disconnection reason semantics
3. In LSM Operational event 40, what internal path emits disconnect reason code `0` for our branch, and what upstream status commonly drives it?
4. Is reason code `0` expected for benign disconnect only, or can it mask an upstream failed post-auth transition?

### C. Next smallest probes (now ETW-capable)
5. Recommend up to 3 minimal probes that use this restored ETW path to distinguish:
   - winlogon/LSM post-auth gate failure,
   - listener/session-class gating,
   - or provider callback ordering influence.
6. For each probe, provide one strict-harness disconfirming observable and the exact ETW event/ID sequence expected to flip.

## 5) New artifacts this cycle

- `artifacts/ida-round12-step1-lsm-etw-baseline-run1.log`
- `artifacts/LSMTraceRound12-20260227-163709.etl`
- `artifacts/LSMTraceRound12-20260227-163709.csv`
- `artifacts/ida-round12-step2-tsstate-etw-baseline-run1.log`
- `artifacts/TSStateTraceRound12-20260227-163852.etl`
- `artifacts/TSStateTraceRound12-20260227-163852.csv`
- `artifacts/ida-round12-step3-lsm-rcm-operational-window.log`
