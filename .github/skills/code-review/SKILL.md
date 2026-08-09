---
name: code-review
description: Review IronRDP pull requests and diffs using focused protocol, compression, prose, and skeptical passes. Use whenever Copilot code review is requested or a proposed change needs review.
---

# Code review

Inspect the diff, its stated goal, and repository guidance before selecting reviewers:

- Run `protocol-reviewer` only when the change may affect protocol behavior, wire data, negotiation, state transitions, security, codecs, virtual channels, or protocol-visible errors.
- Run `code-compressor` for the changed code.
- Run `docs-compressor` when documentation, rustdoc, explanatory comments, or instructional prose changes.
- Run `prose-verifier` when hand-maintained prose changes in a format where source line breaks are stylistic.
- Run `skeptical-reviewer` last for every diff.

Prefer one `rubber-duck` sub-agent per applicable skill.
Run the protocol, compression, and prose reviews in parallel.
Give every agent the change goal, review scope, base and head references, relevant repository guidance, and an instruction to invoke its named skill, inspect the diff itself, make no edits, and return only evidence-backed findings.
If sub-agents or skill invocation are unavailable, apply the same skills directly and keep the ordering.

Before the skeptical pass, normalize the initial results.
Drop claims that lack a concrete location or mechanism, merge findings with the same root cause, and retain disagreements instead of resolving them by vote.
Give the skeptical reviewer the diff plus the retained findings and ask it to independently review the change, verify or reject each supplied concern, and identify material issues the focused passes missed.

Evaluate the final evidence yourself.
Report correctness and protocol findings first, ordered by severity, then optional compression suggestions.
Each retained item needs a location, concrete impact, and actionable correction or simplification.
Merge duplicates across reviewers and do not inflate a maintainability preference into a defect.
Briefly name skipped reviewers and why; if nothing material remains, say that no findings were identified.
