---
name: test-oracle-reviewer
description: Judge whether a test's expected value is justified independently of the code under test. Use when reviewing tests added or changed in a pull request, auditing a module's tests, or whenever expected bytes, snapshots, or mock values may have been copied from the implementation's own output.
---

# Test oracle reviewer

A test whose expected value came from the code under test detects change, not bugs.
For each test, ask whether the expected value can be justified without running the code.

Independent sources: a spec example, an external capture, a hand-derived value, an input the test chose itself and asserts was honored, a real property such as round-trip or `encode` writing exactly `size()` bytes, or a reference model.
Self-derived sources: a snapshot of current output, a value computed through the code or a shared helper, a production constant or message copied into the test, a mock that returns what is then asserted, `is_ok()` or `!is_empty()` on an input that no requirement says must succeed, or a restatement of what a derive or the type system guarantees.

Verify instead of guessing.
Check comments and `git log -p` for the origin, then check the value against the governing spec section: decode a byte vector field by field and show the breakdown, or cite the specification section that defines the behavior.
A value pasted from the code's output is independent once it checks out against the spec, and a spec vector nobody re-read is not; verification is the property, origin is only a hint.
A search-engine or AI summary is not a source; cite the primary document.

Classify each test as independent, self-derived, or unknown, with evidence, and report per assertion when a test mixes classes.
When a value checks out, add a one-line citation comment to the test, such as `// <spec> §<section> example` or `// hand-derived; see PR #N`, so the next review is cheap.
When a value is wrong, report a bug; do not adjust the value or the code.
Unknown is an outcome, not a guess in either direction.

In a pull request review, comment on each self-derived test with the independent source to use and leave the fix to the author.
When that source needs a small non-test change, such as an injectable deadline the test can choose, request the change.
For mutation evidence on the same pull request, use `test-pruner` in pull request mode.
