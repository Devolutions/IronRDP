---
name: grill-intent
description: Round-based interview in which the human writes each design commitment in their own words and the agent supplies context, consequences, and verification, then drafts INTENT.md or *.intent.md with provenance. Use when the user wants to think through or bootstrap intent files rather than have them written from code or a brief.
---

# Grill for intent

Intent is the human's (`intent-files`): the human writes each commitment, and the agent's part is to find the decisions, give the context each needs, verify what the human decides, and tidy the wording to its conventions.

## The tree

Root questions, from which everything else branches; settle vocabulary first, because exact nouns make most commitments one sentence:

- **Vocabulary**: which terms does this unit coin, what is each, which synonyms are avoided, and what rule does each definition carry?
- **Boundary**: which unit, and what is deliberately outside it?
- **Role**: why does it exist; what is lost if it is folded into a neighbour?
- **Must remain true**: what would make the software wrong even with green tests?
- **Rejected alternatives**: where was one way picked over a reasonable other, and why does the other stay excluded?
- **Non-goals**: what is unsupported or was cut?
- **Properties**: security, compatibility, concurrency and lifecycle, dependency boundaries, extension points.
- **Future**: which directions must not be precluded?
- **Second party**: which other party must agree to anything here, so that a `CONTRACT.md` is warranted, and which interfaces have no such party and stay intent?

## Rounds

The frontier is every question whose prerequisites are settled.
When code exists, open with a short brief as its own turn, before any question: what the component is, what it does today, its backends, callers, and dependencies, in plain language and in the human's vocabulary.
End the brief by inviting the human to ask for more or to confirm it; expand where asked, and ask the first round only after they confirm, since a short brief they confirm still catches the places where their memory and the code disagree.
Ask the whole frontier in one round, numbered, with the context the round needs as annexes below the questions: each annex holds the facts for one question or one shared topic, cited with `path:line`, and a question refers to its annex rather than restating the code.
Each question gives the human what they need to decide: the fact it turns on, what the options are, and how each could go wrong later; it does not recommend an answer, because a recommended answer the human accepts is the agent's design under the human's name.
Plain language, checkably: a question names behaviour, not identifiers; identifiers and `path:line` live in the annex; the skill's own vocabulary ("frontier", "closed set") never reaches the human.

```text
❓ **Q1** - **<title>**: <what is at stake, in one or two sentences>
<Today: the fact this question turns on; details in annex A.>
<Options as short labels, each with how it could go wrong.>
<Write your commitment in your own words, or say "no commitment here".>

---

**Annex A** - <topic>: <facts with path:line; alternatives.>
```

Wait for answers, recompute the frontier, repeat.
Look up dependencies, features, callers, and siblings before asking.
Offer options only when they are a closed set the facts define (the variants that exist, yes or no on what the code does today, the list it already uses, two readings of the human's own sentence), as labels rather than finished sentences so that the human writes the sentence, and always include "no commitment here".
Alternatives the human may not know are information, not options: mention them in the annex with what each is good for and where it fails, so the human can widen the choice, and then let them write the commitment.
When the human asks for a recommendation, show what their earlier answers already settle and assemble those words into a candidate sentence for them to accept or rewrite; when they delegate outright, write it and record it as agent wording the human adopted.
When an answer arrives, verify it against the code and the other answers; if it is false today, contradicts another commitment, or has a consequence the human may not have seen, say so with the evidence and ask them to confirm or rewrite before recording it.
Record the human's sentence; tidy it to the conventions only, keeping their words where they are exact, and do not re-present tidied sentences each round, since the final list is where the human confirms them.
Keep a ledger of alternatives: ones the human voiced, options they hesitated between, and any you hold strongly, each with its reason, so that the rejected ones reach the intent file as rationale and the unsettled ones can become an experiment.

## Grill like a reviewer

- An absolute: ask which boundary it protects and what falsifies it; show the sibling, feature flag, or build that would, and let the human scope it.
- Wishing or future tense: ask whether the behaviour exists; if so, ask for the sentence as a requirement, otherwise for the deferral marker.
- A tentative want ("I'd like", "I have in mind", "may"): ask whether it is a requirement, a permission, or a preference, and for the sentence that says which.
- A rule without a reason: ask what alternative it excludes and why.
- A list of variants: ask whether the rule is the list or a property each variant satisfies.
- A fuzzy or overloaded word ("account", "session"): show where the code and the conversation use it differently, and ask for the canonical term and the avoided ones.
- "Requires a decision", "run X after Y", anything about contributors: process; note it for `AGENTS.md` and ask for the software property behind it.
- Exact detail (fields, limits, status codes): note it for a `CONTRACT.md` if a second party must agree to it, and return to the decision behind it.
- A decision about a sibling component: note it with its destination file for the handoff, and return to this component.
- Design still moving (names, packaging, two shapes): say so and keep grilling; do not draft mid-churn, since every rename re-edits every file that carries the name.

Record provenance throughout at the four levels `intent-files/references/handoff.md` defines.

## Finish

When the frontier is empty: list each commitment in the human's words with its provenance, propose placement, and ask for confirmation.
A question no fact settles and the human cannot settle on paper is offered as a `variants` experiment, with candidates from the ledger, criteria, and budget; they may run it, decide now, or say "no commitment here".
Record a question sent to a run as pending; when its brief returns, the human adopts, rewrites, or rejects a candidate sentence before anything is recorded in the draft.
Then draft, write the handoff note from `intent-files/references/handoff.md`, and hand the assembled file back for the human to confirm ordering, placement, and tidying.
