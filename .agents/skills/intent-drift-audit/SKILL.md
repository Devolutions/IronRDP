---
name: intent-drift-audit
description: Audit code, a diff, or a PR against the applicable INTENT.md, *.intent.md, or CONTRACT*.md and report conforms, violates, partial, not-yet-realised, or cannot-verify per statement with file:line evidence, ready for a PR comment. Use once an implementation claims to realise its intent, for conformance checks, automated review, or before modifying governed code; not mid-implementation.
---

# Intent drift audit

Intent files are defined in `intent-files`; they and contracts hold even when tests pass, so this audit reads code against each statement with evidence instead of trusting the suite.
It reports; it never edits intent or contract and never silently fixes code during a review.

## Scope

Paths under audit: the diff, the named component or module, or the subtree.
State which revision you audited; if the checkout may be behind, fetch intent and code from the remote default branch or the PR head.
Applicable intent is what `intent-files` Placement makes cumulative for each audited path.
Where a statement says "see `CONTRACT.<name>.md §<section>`", the contract section is the statement; when the user names a contract, audit an implementation against it directly, treating each item `contract-authoring` requires per operation as a statement and the conformance suite's coverage of each as part of the evidence.

## Verify

Split each file into normative statements, one per bullet or sentence, skipping purpose prose; a sentence about an external producer or environment is context for the statements that depend on it, not a statement.
Classify by meaning, not position: a sentence that constrains the software is a statement wherever it sits, including a glossary definition that carries its rule ("Eject removes credential material from memory, best effort"); only purely explanatory text is context.
Statements in an ancestor file that govern code outside the audited paths are out of scope: skip them and count them in the header.
Give each an ID of its section's anchor, or its number or heading where it has none, plus an ordinal (`CONTRACT.agent-identity.md#channel-2`, `xtask/INTENT.md#requirements-1`), so a later audit can diff against this one even after a heading is reworded or a section renumbered.
A statement is a target, not a requirement, only when it describes behaviour as not yet built ("TPM keys are wanted later"); "for now" on a present constraint ("remains a CI-only check for now") and a `NOTE:` stating an anti-requirement are requirements today, so read the sentence, not the marker; realising a target anyway is worth a note.
It runs on a merged tree or a branch whose implementation is finished; mid-implementation, say so and stop, because every unfinished statement would read as a violation.
For each: cite `path:line`; decide **conforms**, **violates**, **partial** (honoured on some paths, configurations, or the primary path and bypassed on another), **not yet realised** (a deferred target), or **cannot verify** (say what is missing); for a diff, mark **introduced**, **fixed**, or **pre-existing**.
Statements that need design judgement rather than code reading are audited only when the diff plainly bears on them, and labelled as judgement, since they have no `path:line` to cite.
Note `INVARIANT:` and `UNTESTED:` comments that relate; they are the code's own claims.
"Probably fine" is cannot verify: a status of conforms needs `path:line` or a check that establishes the property, not the absence of an obvious counterexample.
When a statement's span is feature-gated or build-specific, check each variant.

Also report **intent rot** (names, paths, sections the intent cites that no longer exist, and plan tense for behaviour that now exists) and **not covered** (behaviour no statement decides: in a diff, new public surface, dependency, fallback, or configuration; in a module audit, any path that looks like a design choice or a latent defect).

## Report

```md
## Intent drift audit: <scope>

Audited against: `<file>` (<n> statements, <m> out of scope), ...

| ID | Statement (abridged) | Status | Evidence | Note |
|---|---|---|---|---|
| xtask/INTENT.md#dependencies-1 | Only lexopt, anyhow, xshell, tinyjson | Violates (introduced) | `xtask/Cargo.toml:14` | adds `serde_json` |

### Violations
### Partials
<Per entry: what intent requires, what the code does, smallest conforming change as a proposal.>

### Cannot verify
### Not yet realised
### Intent rot
### Not covered by intent
```

Lead the table with violations and partials, then cannot verify, then not yet realised, then conforms; drop empty sections.
A clean audit is two lines plus the table.
When the human wants the mapping kept, the table becomes an `EVIDENCE.md` beside the intent, one row per claim with the directive, intent line, and code paths that realise it and `[unsure]` on inferred rows.

When the audit runs because you are about to change governed code, a violation you would introduce is a stop: surface the statement ID, evidence, and options to the human.
