# Criteria

A criterion measures something the design commits to and cannot polish away later; anything a lint catches or the final implementation fixes in a line - panics, licences, formatting - measures nothing about the design and is not a criterion.
Measure with the tool, never by estimate, and the same tool on every variant, or the numbers do not compare.

## Must-pass

A variant that fails one is disqualified from winning; its measurements stay in the brief.

- The acceptance checks, written before the variants and shared by all, pass in full; a count of passes is not a rank, and a test a variant wrote for itself does not count.
- Adopted intent and the contract hold, clause by clause, judged on the probe.
- In an experiment, the decision under test was exercised, not routed around; a probe that makes the decision irrelevant has answered nothing, however clean.

## Default ranking set

| Criterion | Measured by | Trap |
|---|---|---|
| Code size | Non-test lines and new items (types, traits, modules); lines changed in existing code against lines added | Smaller by omission scores well; the gate guards only what the checks exercise |
| API surface | Public items at the seam, their parameters, generics, and lifetimes; the caller for the common case, written out | Count what a caller sees, not what exists |
| Self-containment | Existing items the new code references, excluding std and its own; definitions the reviewer had to open to follow it | Self-contained by duplication: a reimplemented helper scores well unless the reimplementation is recorded against it |
| Reviewability | Whether the reviewer can reconstruct the change from the diff alone and it matches the stated goal; files in the diff | Finding counts penalise novelty, so the most different variant loses by construction |

Self-containment is the inward dependency, what a reader must already know; locality below is the outward footprint, what the change touches; a variant can score well on one and badly on the other.

## When the intent asks

| Criterion | Use when | Measured by | Trap |
|---|---|---|---|
| Extensibility | The intent's future names a direction | Sketch that next change on each probe; files and lines touched | A future you invent measures your preference |
| Locality | The component has neighbours | Files and modules touched outside the component; dependency edges added | - |
| Dependencies | A variant adds a crate | Transitive crates not already in the tree; features enabled; duplicate versions | - |
| Performance | The intent or contract states a bound or names a direction | Margin on a fixed workload; allocations; compile time; binary size | A workload chosen after seeing the variants |
| Test-only access | Behaviour may be hard to observe | `#[cfg(test)]` items, widened visibility, `#[doc(hidden)]`, mocks of internal traits added to the component; fewer is better | Zero by not testing scores the same; the gate is what stops it |
| Failure handling | The contract names failure cases | Cases a caller can distinguish at the seam, the match arm written out; whether an error carries what locates the cause | Panic counts belong to the lint, not here |
| Compatibility | Existing callers, config, or wire format | Call sites changed; migration needed or not | - |

Three to five criteria, each able to split the candidates.
A criterion stays in the brief only if it splits them and measures what its name says; a metric that reads worse as the design improves - setup lines rising because the implementation stopped leaking internals - is mislabelled, not a trade-off for the human to weigh.
No weights: report per criterion, and let the proposed design state the trade it makes, because weighting is the human's judgement.
