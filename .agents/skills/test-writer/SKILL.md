---
name: test-writer
description: Design tests that catch bugs, choosing between an example test, parametrized cases, a snapshot, a property, and a fuzz target, in Rust (`rstest`, `expect-test`, `proptest`) or C# (xUnit `[Theory]`, CsCheck). Use whenever adding or changing unit, integration, snapshot, or property tests in Rust or C#.
---

First name the bug the test catches that existing tests miss; without one it costs runtime and maintenance and guards nothing.
Take the expected value from a source independent of the code under test, as `test-oracle-reviewer` defines; a value derived from the implementation detects change, not bugs.
Mark that source as `test-oracle-reviewer` defines, and assert only what it decides, because an assertion past it pins how the code works today; add one only for feedback while implementing, marked `scaffold`, and never in place of a decision `tdd` says to ask for.
Test at the seam `tdd` defines, the public boundary where the behavior is observed.
A test that fails while the code still does what its marked source requires asserts past that source: narrow it to what the source decides, or delete it unless `test-pruner` protects it; a snapshot that fails only on representation is re-read and updated instead.
Reach nothing beyond loopback, because an external host makes a failure unreproducible.
Each rule here takes a form per language: `references/rust.md` for Rust, `references/csharp.md` for C#; load only the language in scope.

## Properties

Look for a property before writing example tests, because a property holds across generated inputs while examples show only the cases someone thought of.
Candidates are a round-trip through encode and decode, a size computation matching the bytes written, and an invariant from the applicable intent or contract file that speaks of inputs or state, such as one that must survive any sequence of operations.
Reuse the existing generator for each type and add new ones beside it, so each type's generator is written once.
With no property that must hold across many inputs, write example tests; a property whose assertion re-implements the code under test checks the code against itself.
A parser of untrusted input gets a fuzz target rather than a never-panics property, because the fuzzer is coverage-guided and keeps a corpus, which a few hundred random cases are not; where no fuzz harness exists to add one to, a property over malformed inputs stands in, and a pull request comment says the parser is not fuzzed.

## Snapshots

Snapshot structured values such as a parsed config or a decoded message, with the expected value beside the assertion.
Taking or updating a snapshot is the oracle, so read and check it, because its value is whatever the code produced; when a spec or capture defines the value, assert that instead.
Move a snapshot or blob longer than a screen into a sibling file, so the source shows the test rather than its data; keep short byte vectors inline so each field can carry its comment.

## Examples and cases

Parametrize only when every case checks the same behavior with different data, and name each case so a failure names the case, not an index.
A body that branches on its parameters, through a conditional or an optional expected value, is several tests behind one name, so a failure reports a case instead of the behavior that broke; write one test per behavior and share setup through a helper.
A cross product of parameters is right only when every combination is a real scenario; one that needs a conditional to skip or adjust a combination is that branching.
Keep each regression test standalone and named for its issue, because its setup reproduces the bug's exact conditions and a case table would generalize them away.
