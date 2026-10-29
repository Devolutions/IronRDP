---
name: blind-testsuite
description: Write or extend a conformance suite from a CONTRACT*.md and published sources only, by an author who never sees an implementation, kept read-only while implementations are built against it. Use when a contract has or will have several implementations, or a wrong expected value would be invisible to the implementer, as in cryptography, codecs, and wire protocols.
---

The suite is independent evidence derived from the contract, not the definition of conformance, so a failing test says the implementation or the test is wrong, never the contract.
`test-writer` decides each test's shape and `test-oracle-reviewer` what counts as an independent value; this skill is the isolation around them.

## What the author sees

The contract file, the interface it is written against (the contract crate or module, its types and documentation), and published sources: specifications, vendored vectors, captures.
Not an implementation, an earlier implementation, or the pull request that produced the contract, because each carries a reading of the contract the suite would then confirm rather than test.
When an implementation exists, the author is a fresh context in a worktree at a commit without it; a do-not-read list supplements that and does not replace it, since a context that has seen the implementation cannot unsee it.

## Expected values

From a published source cited in the test, a property the contract states, or a derivation whose method the contract or cited standard fixes, recorded beside the vector so a reviewer can redo it; never from a computation only an implementation of the contract could perform, because checking such a value means implementing the contract again.
A derived vector accounts for every coupled field the change touches - length, checksum, signature - or its outcome is not what the derivation claims.
Where no published answer exists, compare outputs across implementations; a differential check never stands alone, because a bug shared by both passes it.
A hand-written negative input differs from an independently valid control in the targeted dimension only, so the test can fail only for the reason it names; a rule that names a rejected input gets one, since such a rule tested only by acceptance is met by accepting everything.
Vendored sources keep origin, version, and checksum recorded, are taken whole or cut at their own section boundaries, and are never edited, because an edited or sliced vector loses the provenance that makes it an oracle.
Keep a coverage record beside the suite: which test checks each clause, which clauses are verified by review only, and the boundaries deliberately left unchecked, so silence is not taken for coverage.

## Review and merge

The reviewer runs `test-reviewer` and checks the contract against the coverage record both ways: every clause has a test or a recorded gap, every test cites its clause.
Write the suite before the implementation; merge it once one implementation runs it green, so a test no conforming implementation can pass is found before it binds every later one.

## Read-only during implementation

An implementation pull request never edits the suite or the contract.
A test that looks wrong, or a clause with no test, is escalated with the vector or clause, expected and observed values, and the cited source.
The fix goes to the pull request that owns the suite - the open one through its lead, or a new one once it has merged - by a fresh author with the same inputs, who receives the decision and not the implementation's output, so the suite stays blind.
