---
name: variants
description: Run two or three isolated probe implementations of one brief that differ in named decisions or approaches, judge them blind against fixed criteria, and return a decision brief from which the human or the lead picks or composes the final design. Use inside a delivery or intent flow once the alternatives are named and only implementation evidence can settle them, or when the user asks to try several approaches.
---

A variant is a probe: build it only far enough to measure the criteria and never merge it, because probe code carries shortcuts and has not been through the review loop; the chosen design - one variant, or the best decisions of several - is implemented fresh and the criteria rerun on it.
A run answers a question nobody could settle on paper; when the facts or the human can settle it, ask instead, since three probes cost more than one question.

## Two modes

- Tournament: intent and contract are fixed and the candidates are approaches; whoever runs it picks, within autonomy.
- Experiment: the candidates are decisions the human has not committed; the human picks and writes the commitment, so no intent file changes until they do.

Never run a candidate that contradicts adopted intent; that is drift with a budget, not an experiment.

## Compose

Candidates come from alternatives the human voiced, the options of an escalation, or alternatives you derive and name before launch; without a named difference there is nothing to run.
Vary one decision when the open decisions are independent; when they are coupled, make each variant a coherent design - a full set of answers - and compare designs, because a bundle that mixes answers at random tests nothing.
Three variants by default, raised only by the human, because each costs a build.
Fix the criteria before the first variant starts, choosing from `references/criteria.md`, each with how it is measured; the acceptance checks are written then and shared, since a check a variant writes for itself proves only that it did what it did.
When no criterion can distinguish the candidates, return the question to the human rather than inventing a proxy.

## Run

Same base commit, same brief apart from the named differences, same model for every variant; these reduce what else could explain a difference, without proving the design caused it.
Each variant in its own worktree or session, never shown another's work, or they converge.
The contract and any blind suite stay read-only in every variant, and a defect found in them is escalated outside the run, because a candidate that moves its oracle is not comparable.
A variant stops once every criterion can be measured on it, or earlier when its candidate proves infeasible, since the run has answered for it; one that fails a must-pass criterion is disqualified from winning but keeps its measurements, since they still inform a composed design, and when all fail, investigate before concluding anything from the run.

## Judge

A fresh context, from a different model family when available, gets the criteria, the measurements, and snapshots labelled A, B, C stripped of branch and commit metadata; never who or which model built which, the workers' transcripts or rationales, or your preference or proposed design, because each of those carries the framing the blind read is meant to remove.
It ranks per criterion with the evidence it turned on, names an attribution to one decision only where the evidence isolates it, and reports bundled effects as unidentifiable otherwise.
A variant another beats on every criterion that splits them is dropped, so the brief carries only the front where a trade exists; a criterion that split nothing is dropped from the brief too.
"No clear winner" is a verdict; so is "none of these", with the decisions a better design would combine.

## Brief and after

Write the decision brief from the template as an artifact, never committed; it is what the human reads instead of three implementations.
Keep the variant branches until the design they informed has merged, since the final implementer may read how a probe handled a decision; then delete them, because the brief is what survives.
In an experiment, proceed on the proposed design while the human is away only when the verdict is clear by the criteria, logged as provisional; the human adopts or reverses from the brief, at the latest when they merge, since nothing resting on a provisional decision merges without them.
A verdict that is not clear stops this thread and waits; other threads continue.

```md
# Experiment <n>: <question>
Base: <commit>. Variants: A <decisions>; B <decisions>; C <decisions>.
Criteria: <each with how it was measured; must-pass marked>.
| Criterion | A | B | C |
Verdict (blind): <ranking; evidence; attributions the evidence isolates; effects left unidentifiable>.
Per variant: <what it demands of callers; what it forecloses and the cost of switching later; surprises met>.
Proposed design: <a variant, or the decisions combined>, because <evidence>.
Status: proceeding provisionally | waiting.
To adopt or rewrite: <one candidate commitment sentence per decision>.
```
