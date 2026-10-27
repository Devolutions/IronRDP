---
name: contract-review
description: Review a normative CONTRACT*.md for overreach, unspecified failure paths, duplicated lists, examples standing in for rules, one-implementation bias, and untestable wording; returns findings with proposed rewordings, never edits. Use when asked to review, critique, tighten, or prepare a rewrite of a contract or protocol specification.
---

# Contract review

A contract is implemented and tested by parties who read only it, so every ambiguity becomes a divergence between implementations.

## Gather

State the revision reviewed.
Collect the contract, the `INTENT.md` it serves, the conformance suite or test vectors derived from it, and each implementation.
Run every state-changing operation through the gap questions in `contract-authoring/references/critique-lenses.md`; `contract-authoring/references/patterns.md` lists what earlier reviews had to fix.

## Lenses

Apply each to every normative sentence and quote it, so the author finds it after edits.

- **Party**: which second party in scope must agree to this sentence?
  None means it is one party's own realisation (config, storage, CLI) and belongs in that party's intent file.
- **Intent contradiction**: the contract says what its `INTENT.md` excludes, or excludes what the intent commits to; detail beyond the intent is expected, disagreement is not, and which side is wrong is the human's call.
- **Stage talk**: "V1", "initially", "will be", "for now", or a party named with a stage ("A now, B later" for "e.g.: A, B"), since a release stage is not a rule a party agrees to; the contract states the whole surface, and what is realised today is the co-located intent file's deferral.
- **Overreach**: a claim stronger than the mechanism guarantees ("prevents oracles" for "does not say which check failed"); propose the narrowed form.
- **Open set**: an error, outcome, or input class not drawn from a closed, defined set; "bad input" is not a classification.
- **Order for precedence**: an evaluation order where only which-outcome-wins matters, or two outcomes equally valid for one input.
- **Unowned field**: a stored field with no statement of who initialises it and which operations update it or must not.
- **Second copy**: a list, mapping, or encoding defined twice; keep one and point.
- **Anchor**: an operation or normative section without an anchor, an id repeated within the file, a published anchor renamed, or an anchor removed while citations still name it, since the suite, the code, and the intent cite sections by anchor (`intent-files` Anchors).
  Propose the id or restore the old one; for a removal, list the citers a search for the id finds.
- **Example as rule**: a transcript or template whose free aspects (order, case, whitespace) are not stated; propose the statement and vectors that vary them.
- **One-implementation bias**: a library's behaviour, a number, or a rationale true for one backend; ask whether it was written to fit one implementation.
- **Unmeasured cause**: a rationale citing a mechanism nobody measured.
- **Untestable**: "handles", "supports", "properly", a condition without a stated outcome, or two outcomes in one sentence; propose an observable with a bound and the conformance case that checks it.
- **Scope creep**: admin, UI, mock control, implementer choices, a conformance section or a "must pass the vectors" rule (test vectors and suites are evidence for the rules, not rules), an acceptance rule for the work such as "passes the suite before merge", or any section that neither constrains a party in scope nor is exercised by a conformance test; propose the owning component's intent file, the suite's coverage record, the process rules, or deletion.
- **Intent gap**: a contract rule that takes a design decision `INTENT.md` does not, as opposed to adding detail; the human records the decision in the intent first, since a decision made only in the contract is one nobody made.

## Report

Order by consequence: party, intent contradiction, and gaps, then overreach, open sets, second copies, and anchors, then wording.

```md
### <N>. <lens> — §<section>

> <quoted sentence>

<Why, citing an implementation, a test, or the intent when relevant.>

**Proposed:** <reworded sentence>, or move to <section or artifact>, or delete.
```

Close with what is strong as written, the questions only the human can settle, and a verdict: freeze, freeze after corrections, or rewrite.
