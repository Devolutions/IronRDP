---
name: intent-review
description: Review INTENT.md and *.intent.md files or drafts for contradictions, unscoped absolutes, misplaced content, rot, and missing rationale; returns findings and proposed rewordings, never edits. Use when asked to review, check, or tighten intent files.
---

# Intent review

Intent files are defined in `intent-files`; agents read them literally before every change in their directory, so a loose sentence becomes a wrong implementation.
A proposal rewords a commitment the human already holds; when a finding can only be fixed by deciding what the software should be, ask instead of proposing.

## Gather

State which revision you reviewed; if the checkout may be behind, fetch from the remote default branch and say which findings apply to which version.
Collect every file `intent-files` Placement makes applicable and the `CONTRACT*.md` it defers to; contradictions mostly live between files, and copies of a part file that differ are a finding on their own.
Read the governed code, manifest, features, and configurations; most false absolutes are found there.
When a sentence feels wrong and you cannot name why, `intent-files/references/pitfalls.md` is the checklist.

## Lenses

Apply each to every normative sentence and quote the sentence in the finding, so the author finds it after edits.
Four probes decide most of them: is this authoritative or only explanatory; would changing it require a design decision; would it still matter after a rewrite; what undesirable implementation could still satisfy it.

- **Contradiction**: two statements that cannot both hold, including a specific file quietly narrowing a broader one.
- **Not a commitment**: rules out no plausible change, would not survive a rewrite, or restates code, manifest, README, path, contract, or a sibling without adding a constraint or reason not recoverable there; at section level, scope prose that placement already supplies, boundary notes repeated, headings for a handful of bullets nothing cites, inventories whose owner is elsewhere; "does that belong to this component?"
- **Still satisfiable badly**: a commitment an undesirable implementation meets as written; propose the wording that excludes it.
- **Process in intent clothing**: "requires a decision", "run X", anything about agents or contributors; propose the software property behind it, or `AGENTS.md`.
- **Plan or tentative tense**: "we would like", "will be extended", "later" for behaviour that already exists, since a realised commitment must read as normative; "I have in mind", "may be used" for a firm want; propose the normative form or the explicit permission.
  A deferral the code has not realised is correct as written, not a finding.
- **Schedule on parties**: a party or example named with a stage ("A now, B later" for "e.g.: A, B"); timeline is not intent.
- **Unscoped absolute**: `never`, `always`, `only`, `every`; which boundary, and does a sibling, feature flag, backend, or build falsify it?
- **Overstated**: `must`, "if and only if", "exactly these", an exhaustive list, with nothing showing the strength is intended; ask before proposing the weaker form.
- **Unverified universal**: "every library does X" with no cited check.
- **Code disagrees**: the code does not do what the sentence says; report it as a divergence for the human, not as a defect in either.
- **Enforced but unwritten**: a wire format, state machine, or dependency direction the code enforces that no document states; propose where it belongs.
- **Incidental detail**: a helper name, layout, or one library's behaviour or number stated as a rule with nothing showing a human chose to pin it; a deliberately pinned detail with its reason is a commitment, so ask rather than flag, and an author's insistence that it is intentional settles it.
- **History for rationale**: "previously Gateway was stateless, which caused problems with deletion" where the intent is "deleting an agent survives Gateway restarts"; propose the property.
- **Missing reason**: a rule a contributor could plausibly "fix" because no alternative is named as excluded.
- **Not checkable**: "handles", "supports", "properly", a condition without a stated outcome, or two outcomes in one sentence; propose an observable with a bound.
- **Anchor**: a section a test, code comment, or document cites without an anchor, cited rules in a flat list with no heading, an id repeated within the file, a published anchor renamed, or an anchor removed while citations still name it, since each breaks or blocks a citation (`intent-files` Anchors); search the tree for the file's name to find the citers.
- **Hygiene**: links and anchors that do not resolve, removed rules still named, a synonym the glossary avoids, duplicated sentences across siblings, house style.

## Report

Order by consequence: contradictions and false absolutes, then not-a-commitment and process, then anchors and hygiene.

```md
### <N>. <lens> — `<file>`

> <quoted sentence>

<Why, citing code or another file when relevant.>

**Proposed:** <reworded sentence>, or move to `<artifact>`, or delete.
```

Close with what is strong as written, the questions only the human can settle (two plausible readings the code does not decide, which of two files is right, what they want preserved where a proposal would change the design), and a one-line verdict: adopt, adopt after corrections, or rework.
A clean file gets a short report.
