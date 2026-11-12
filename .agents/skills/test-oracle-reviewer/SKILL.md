---
name: test-oracle-reviewer
description: Judge whether a test's expected value is justified independently of the code under test, and define the `oracle:` marker that records its source. Use when reviewing tests added or changed in a pull request, auditing a module's tests, marking a test's source, or whenever expected bytes, snapshots, or mock values may have been copied from the implementation's own output.
---

A test whose expected value came from the code under test detects change, not bugs.
For each test, ask whether the expected value can be justified without running the code.

Independent sources: a spec example, an external capture, a hand-derived value, an input the test chose itself and asserts was honored, a named law such as round-trip, a property an intent, contract, or spec states, such as `encode` writing exactly `size()` bytes, or a reference model.
Self-derived sources: a snapshot of current output, a value computed through the code or a shared helper, a production constant or message copied into the test, documentation in or generated from the code under test, such as its rustdoc, a trait's included, or an OpenAPI file generated from its annotations, a mock that returns what is then asserted, `is_ok()` or `!is_empty()` on an input that no requirement says must succeed, or a restatement of what a derive or the type system guarantees.

Verify instead of guessing.
Check comments and `git log -p` for the origin, then check the value against the governing spec section: decode a byte vector field by field and show the breakdown, or cite the specification section that defines the behavior.
A value pasted from the code's output is independent once it checks out against the spec, and a spec vector nobody re-read is not; verification is the property, origin is only a hint.
A search-engine or AI summary is not a source; cite the primary document.

Classify each test as independent, self-derived, or unknown, with evidence, and report per assertion when a test mixes classes.
When a value checks out, add its marker, as below, so the next review is cheap.
When a value is wrong, report a bug; do not adjust the value or the code.
Unknown is an outcome, not a guess in either direction.

## Markers

Mark where an expected value comes from with an `oracle: <kind> <reference>` line comment of the test's language, such as `// oracle: …`, on the test, on a case row, or on one assertion when a test mixes sources, so one search finds every test that rests on a source.
Strong kinds name an independent source, and a test, case, or assertion marked with one is deleted only when its source changes or is shown not to decide what it asserts:

- `spec <document> §<section>`: normative text other than a contract or intent file, such as an RFC, a Microsoft open specification, or `docs/JMUX-spec.md`.
- `contract <path> §<section>`: a `CONTRACT*.md`.
- `intent <path> §<section>`: a statement in an intent file.
- `regression <issue or crash file>`: a bug report, as a Jira key such as `DGW-123` or a GitHub issue such as `#123` or `owner/repo#123`, or a fuzz crash file under `fuzz/`.
- `reference <implementation, capture, or model>`: what another implementation, a capture, or a reference model does, recorded or compared live; never the code under test or its own documentation, because documentation written with the code describes what it does rather than deciding what it must do.
- `property <law>`: a law of the code under test named by one of `round-trip`, `idempotent`, `involution`, `commutative`, `associative`, or `monotonic`.

Write every path from the repository root, with forward slashes and exact case, such as `crates/gateway-db/INTENT.md`, because markers sit at every depth and a root-relative path reads and resolves the same from each.
A file in another repository is cited as `<owner>/<repo>@<sha>:<path>`, with `<path>` from that repository's root, such as `Devolutions/devolutions-gateway@43958682a1b2:docs/agent-identity/CONTRACT.agent-identity.md`, and `<sha>` a commit of at least 12 hex digits, never a branch or tag, because a pinned commit never changes and so resolves the same every time.
A `spec` document outside `docs/`, such as `RFC 6455` or `MS-RDPBCGR`, is named rather than given as a path.
`§<section>` names the `id` of an anchor the file places before a heading as `<a id="…"></a>`, or else a heading, in any case, or the number a heading starts with, so `§3.2` matches `3.2 Framing`.
Cite the anchor when the file has one, because it survives a renamed heading or a renumbered section.
When a pinned section changes upstream, re-check the test against the new text, then bump the pin or change the test, never the expected value alone, because a value changed without its pin cites text that decides the old one.

A `property` law is accepted only by its well-known name, because the name is its definition, while any other property needs a written source: it takes the kind of the document that states it, and is `scaffold` until one does.
An assertion whose expected value comes from running the code under test is self-derived even over generated inputs, so it is `mutant` or `scaffold`, never `property`.
A codec's `round-trip` test never replaces its spec or capture vectors, because a round-trip passes symmetric bugs.
A hand-derived value, a checked snapshot, or an input the test chose and asserts was honored takes the kind of the source that requires it.

Weak kinds name a self-derived source, which detects change rather than bugs, so any intentional change, such as a refactor or a new feature, may delete a test, case, or assertion marked with one without justification unless `test-pruner` protects it:

- `mutant <faults in words>`: a test or assertion kept to catch mutants that no strong source decides.
- `scaffold`: a test or assertion written for feedback while implementing that no strong source decides.

No `scaffold` marker merges: before the pull request does, give what it marks a strong source, keep it as `mutant` when `test-pruner` finds it alone catches a fault, or delete it.
When it checks a property or intention no document states, ask whether and where to specify it, and mark it with that source once specified, since a confirmation alone is not a source; a protected test takes this path rather than deletion.

Mark a strong source whenever one decides the value, so a test found through a mutant but decided by a spec is `spec`, because whether a test may go depends on whether a source still requires it, not on how it was found.
An unmarked test starts as unknown: verify it as above, and turn a citation in another form into the marker it supports.
A marker is a claim: verify it like any other origin.
A marker citing the code under test's own documentation is self-derived whatever its kind: re-cite the test to the intent, contract, or spec sentence that decides its value, keep it as `mutant` when `test-pruner` finds it alone catches a fault, or delete it.

## Review

In a pull request review, comment on each self-derived test that an independent source decides, naming that source, and on each changed test without a marker; leave the fix to the author.
When that source needs a small non-test change, such as an injectable deadline the test can choose, request the change.
For mutation evidence on the same pull request, use `test-pruner` in review mode.
