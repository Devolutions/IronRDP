---
name: contribute
description: Pick the delivery flow by scale - a test-first fix in place, one led pull request, or an orchestrated multi-pull-request effort - and where the intent, contract, test, and review skills enter. Use at the start of any non-trivial change, or when asked how to approach or stage work.
---

Every flow is the same loop at a different scale: decide what must hold, write the check that fails without it, make it pass, and have someone who did not write it review.

## Pick the smallest flow that fits

| Flow | When | Run |
|---|---|---|
| Fix | One behaviour at one seam; every decision it needs is already in the intent, the contract, or the bug report | `tdd` here, then `code-review` |
| Change | One pull request; may raise a design decision or need more than one pair of hands | `grill-intent` for any decision none of those states, then `lead` |
| Effort | Several pull requests, a second party that must agree on an interface, or a suite written blind | `orchestrate-effort` |

Promote when a signal appears rather than starting big, because each tier adds sessions and context to keep in step.
A decision neither the intent, the contract, nor the report states promotes to Change: stop and run `grill-intent` rather than settling it in a test or code, since the first place a decision lands becomes the text everything else is bent to.
A second party or a `CONTRACT.md` change promotes to Effort: the contract is its own pull request before the code that realises it.

## Where the other skills enter

- `intent-files`: read every applicable intent file before the first edit, in every flow.
- `contract-authoring` and `contract-review`: when two independently built parties must agree; the contract precedes its first implementation.
- `blind-testsuite`: when a wrong expected value would be invisible to the implementer, as in cryptography, codecs, and wire protocols; written from the contract before any implementation.
- `test-writer` for every test, `test-reviewer` and `test-oracle-reviewer` through `code-review`, `dependency-reviewer` through `code-review` when a manifest or lockfile changes, and `dependency-pruner` then `test-pruner` on the diff once the code and tests are complete and before the last review.
- `intent-drift-audit`: once a realisation is claimed complete, in the lead's final check or the reviewer's pass, never mid-implementation.
- `variants`: when a named design choice needs implementation evidence rather than a paper answer.
- `review-feedback-loop`: once a pull request has findings.

## Wrong-flow signals

- An expected value you cannot justify independently of the code under test: the oracle, perhaps the seam, is wrong; see `test-oracle-reviewer`.
- An intent file, `CONTRACT.md`, or blind suite this pull request does not own edited inside it: that change belongs to the pull request that owns the text.
- More open pull requests than you can verify one by one: stop opening new ones.
