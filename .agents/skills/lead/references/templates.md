# Lead templates

Session artifacts: save in the harness's artifact area, never commit.

## Sub-agent brief

```md
# <task id>: <title>

Role: worker | tester | suite author | scout | reviewer.
Goal: <one sentence>.
Acceptance: <checks the lead will run, one per line>.
Scope: <paths the sub-agent may change; none for a scout or reviewer>.
Read first: <applicable INTENT*.md, *.intent*.md, CONTRACT.md, AGENTS.md paths>.
Do not read: <paths excluded for this role, or none>.
Do not: push, open a pull request, or edit outside Scope.
When done: <worker, tester, suite author: commit, report the range and what you could not do, stop editing until the verdict | scout, reviewer: report findings, no edits>.
```

## Lead report

```md
# <pull request>: report <n>

Head: <SHA>; CI: <run URL>.
Non-success jobs: <each by name, or "none">.
Tests: <counts per suite or platform>.
Changed since last report: <file:line summary>.
Review: <verdict quoted verbatim, round count>.
Decided within autonomy: <each, one line>.
Deviations from the brief: <each with reason>.
Decision needed: <question>; options (a) (b) (c) with how each could go wrong; recommendation.
UNTESTED: <properties, or "none">.
Status: waiting | blocked on <what> | ready.
```
