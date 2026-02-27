# IDA Termsrv/RDS Reversing Guidance — Round 11 Questions (2026-02-27)

Based on `artifacts/ida-termsrv-guidance-answers-round10-20260227.md`, we ran the next minimal probes and captured strict/runtime evidence.

## 1) New probe outcomes

### A. `IRDP-Tcp\Execute=' '` (single-space) disconfirming probe
- Registry setup:
  - `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp\Execute = " "` (length=1)
- Strict run log:
  - `artifacts/ida-round10-step2-irdptcp-execute-space-run1.log`
- Observed outcome:
  - Type10 + session-linked graphics proof: **True**
  - `NotifyCommandProcessCreated=1` with callback-time snapshot still pre-userinit (`userinit=false`, `explorer=false`, `logonui=true`, `winlogon=true`)
  - Strict remains the same 2-issue no-shell boundary (blank/uniform frame + `explorer.exe` absent)
  - No new TermService crash event attributable to this run
- Interpretation:
  - Non-empty textual command is likely required to enter the explicit-command crash branch.
  - Whitespace-only appears normalized/treated equivalent to empty for this path.

### B. `RDP-Tcp\Execute='userinit.exe'` with standard RDP client probe
- Probe log:
  - `artifacts/ida-round10-step1d-rdptcp-userinit-domain.log`
- Client path:
  - `cargo run -p ironrdp --example screenshot --features "session connector graphics" -- --host IT-HELP-TEST --port 3389 --username Administrator --domain IT-HELP --password ...`
- Observed outcome:
  - Connection progresses through CredSSP and **licensing completed** (`StatusValidClient`), then times out at capabilities exchange (`os error 10060`)
  - No new `Application Error 1000` TermService crash event after this run
- Correlation note:
  - Latest crash event remains at 15:38:49 (`offset 0x2d326`) from earlier explicit-command testing window.
- Interpretation:
  - For this run, `RDP-Tcp + Execute=userinit.exe` did **not** reproduce immediate termsrv AV, but still did not complete interactive session.

### C. Post-probe baseline revalidation
- Log:
  - `artifacts/ida-round10-step3-postprobes-baseline-run1.log`
- Outcome:
  - Baseline signature unchanged: Type10+graphics+callback present; still pre-shell boundary (2 strict issues)

## 2) Current consolidated state

- **Crash branch** (debugger-backed): explicit command on `IRDP-Tcp` can still trigger termsrv cleanup AV at `termsrv+0x2d326` (`SmartPtr::operator=` in CTSLicense cleanup).
- **No-shell branch** (stable baseline): Type10 and graphics succeed, but no `explorer.exe` launch and blank/uniform frame persist.
- **Disconfirming finding:** `Execute=' '` does not trigger crash branch; command content appears semantically significant.
- **ETW constraint (re-validated):** target VM free space is ~34 MB on `C:`; even minimal `logman` trace (`bincirc`, 8 MB, single LSM provider) fails with `There is not enough space on the disk`.

## 3) Focused round11 questions

### A. Execute parsing / branch predicate precision
1. In `StartSession` / `ITSSession::Start` call chain, what exact predicate is used to treat `Execute` as active command?
   - e.g., null vs empty vs whitespace-trimmed-empty vs tokenized executable parse success.
2. Where is whitespace-only command canonicalized (if at all), and does that happen before listener-specific branching?
3. What is the smallest non-empty command string that must flip behavior from baseline to explicit-command branch?

### B. Listener-specific branch behavior
4. Why does `RDP-Tcp + Execute=userinit.exe` in this run reach licensing completion yet stall at capabilities instead of crashing?
5. Which termsrv function gates the transition from licensing-complete to capability/channel progression for this case?
6. Is there a listener-type conditional that routes `RDP-Tcp` explicit-command failures to timeout/abort while `IRDP-Tcp` can route to cleanup AV?

### C. Crash path prerequisites
7. In the explicit-command crash path, what exact preconditions produce stale SmartPtr fields in CTSLicense (which field(s) remain uninitialized)?
8. Is there a specific early-return status from `ITSSession::Start` / `CWsxMgr` / licensing event sink that bypasses constructor zero-init or post-init sanitation?

### D. No-shell branch convergence
9. What shared machine-wide gate still blocks shell commit after successful Type10 + graphics + callback (with both listeners observed in telemetry)?
10. Which internal state machine edge (LSM/winlogon/termsrv) best explains persistent `NotifyCommandProcessCreated` pre-userinit snapshots?

## 4) Requested next minimal probes (IDA-guided)

Please propose up to 3 highest-signal probes with:
- exact function/branch target in termsrv/winlogon/LSM,
- exact expected observable in our strict harness,
- one explicit disconfirming condition per probe,
- and preference for probes resilient to our current ETW disk-space limitation.

## 5) Artifacts (new this round)

- `artifacts/ida-round10-step2-irdptcp-execute-space-run1.log`
- `artifacts/ida-round10-step1d-rdptcp-userinit-domain.log`
- `artifacts/ida-round10-step3-postprobes-baseline-run1.log`
