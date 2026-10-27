---
name: test-pruner
description: Prune, strengthen, and consolidate tests in a pass of at most one hour, using oracle classification and domain mutants, with cargo-mutants or Stryker.NET where their estimated run fits that hour. Use when asked to remove redundant tests, consolidate tests into cases or properties, audit test value, clean up AI-written tests, finalize a change's tests before its pull request opens, or get a mutation-backed review of a pull request's tests. For a quick oracle-only check, use `test-oracle-reviewer` instead.
---

Scope each pass to one crate, project, or module, or to one change's diff, so results stay reviewable.
Decide from two independent measurements: whether the expected value is justified independently of the code, and which faults only this test catches.
Mutation testing alone cannot tell a test that matches itself from a correct one, and oracle review alone cannot tell which tests are the sole guard for a fault.

Editing mode is for tests you own, such as a change's diff before its pull request opens: commit the changes yourself and hand the evidence tables (oracle table, kill matrix, challenge results, survivors, and found bugs) to whoever posts them as a pull request comment.
Review mode is for another's pull request: post a review and change nothing in its branch, applying mutants and challenge patches only in the clone below.
Run mutants in a clone of their own and address its files by absolute path, since a relative path resolves against whichever checkout the shell starts in, and an applied mutant left in a shared checkout corrupts every other run there.

## Budget

A pass has a wall-clock budget of one hour, mutation runs included, and aims for under 30 minutes, because exhaustive mutation runs over whole areas take hours and buy a point or two of score.
Before starting any mutation run, estimate it as clean scoped test time × mutants ÷ concurrency, plus build time where each mutant rebuilds.
When the estimate exceeds what remains of the budget, narrow the scope to the critical paths or stop and report what the run would cost; never extend the budget silently.
A reviewer's request for more mutation evidence falls under the same budget.

## Measure

Invoke `test-oracle-reviewer` on every test in scope and record independent, self-derived, or unknown with evidence.
The default mutant source is domain mutants written by a fresh-context sub-agent, because a few calibrated domain faults cost minutes where a generic run over the same code costs hours.
Give it the applicable intent and contract files, the code under test, and `references/domain-mutants.md`, but not the tests, so the faults come from what the code must do rather than from what the tests already check.
Ask for one patch per fault, each a single plausible mistake, weighted toward critical paths such as validation, cryptography, parsing, and authorization decisions, and as many as the estimate fits in the remaining budget.
Write one positive control yourself, a mutant that a test you have read plainly catches, and apply it before the sub-agent's patches: if no test fails, the patched code is not what the tests run, as with a stale build or a filter that matches nothing, and every result of the pass is void.
Apply each patch as `references/domain-mutants.md` describes.
Add cargo-mutants (`references/cargo-mutants.md`) or Stryker.NET (`references/stryker.md`) where the estimate fits the budget, critical modules first; load only the language in scope.
Build the kill matrix of mutant by test: for a domain mutant, from the failing tests of its scoped run; for a tool mutant, from the tool's outputs, with per-test reruns only where the reference names them.
A unique kill is a mutant caught by exactly one test in scope.
Zero unique kills over nothing caught measures nothing and lets the invariant pass trivially, so a test whose code no mutant of the pass reaches is challenged rather than deleted on that evidence, until the budget runs out.
For tests with several assertions, record which assertion fails for each kill.
Run the scoped suite a few times on the clean tree to find flaky tests before trusting any kill.

## Decide

Keep independent tests; merge them into cases of one parametrized test only when they check the same property and differ only in data.
Never delete an independent test on mutant evidence: the mutant sample is tiny next to the real fault space, and a spec vector costs nothing to keep.
Strengthen a self-derived test with unique kills by replacing its expected value with an independent one; if none exists, keep it with a `mutant` marker, as `test-oracle-reviewer` defines, naming each fault it alone catches in words, since mutant ids and line numbers change between runs.
A strengthened or gap test that fails is a finding, never a task to make it pass: report the test, the independent expected value and its source, and the code's actual output to whoever assigned the run in editing mode, or to the author in review mode, and change neither test nor code meanwhile, since whichever is edited first becomes the specification.
Delete a self-derived test with no unique kills.
A test of unknown oracle with unique kills has already passed the challenge; strengthen it.
Challenge a test of unknown oracle without unique kills, unless it is protected below, since its fate does not depend on the answer.
No `scaffold` marker outlives the pass: resolve each as `test-oracle-reviewer` requires, sending its questions to whoever assigned the run, or to the author in review mode.

Deletion is a set decision, not a per-test verdict.
Two tests that both catch mutant M each have zero unique kills, and deleting both loses M.
A deletion is proven when the remaining tests still catch every mutant the deleted tests caught.
A test of unknown oracle still without a named defect when the budget runs out is deleted, since `test-writer` requires a named bug; a unique kill counts as named.
Independent and protected tests are never deleted on budget grounds.
The same rule applies to assertions: a self-derived one that never fails across its test's kills is a deletion candidate.

Protected tests are never proposed for deletion: a regression test tied to an issue or fuzz crash, a test of untrusted-input parsing or bounds checks, and a doctest.
That holds whatever their marker, because a refactor of a parser is where a dropped bounds check goes unnoticed.
Treat a suite that deliberately uses snapshots as a change detector as protected: dedupe it, do not delete it.
Report flaky tests separately; deleting one hides a bug.

## Challenge

Give a fresh-context agent the test, the code under test, and the mutants the rest of the suite already catches, and ask for one mutant that this test catches and the others miss, as a patch only.
Apply and run the patch yourself; arguments do not count.
Record any mutant it finds in the evidence regardless of the verdict, and propose a reusable one for `references/domain-mutants.md` in a separate pull request, since neither mode edits skills.
If it finds none, ask the human who wrote or prompted the test for the source of its expected value, and accept a source you can check, never a confirmation.
Without one, delete the test.

## Consolidate

Consolidate into the case and property forms `test-writer` gives for the language, naming every case, since the kill matrix reports each case by its name.
Use a property test only for a real property; an assertion that re-implements the unit is a self-derived test with random inputs.
Round-trip passes symmetric bugs, so every codec module keeps spec or capture vectors.
Do not add a never-panics or never-throws property where a fuzz target already covers it.

## Invariant and output

The caught-mutant set never shrinks except through deletions a human approves in a pull request.
After every deletion set and every consolidation, rerun the mutants the removed or changed tests caught, not the whole scope, verifying by rerunning rather than reasoning, and revert on any loss; compare runs made with identical settings.
After ruling out the scoping errors and tool artifacts the reference names, triage each survivor as equivalent (no observable change), unconstrained (an observable change no independent requirement decides, such as a log message's wording), coverage gap (a change an independent requirement rejects), or bug lead (the original code looks wrong, or is unreachable in every supported configuration).
Close each coverage gap with a test at the seam `tdd` defines, marked with the requirement that rejects the change and written with `test-writer`; a survivor only a test of internals could catch is not a gap but equivalent or a bug lead, as above.
A suite of independent tests yields few deletions; a pass may end in consolidations and gap tests only, or in no change.
Report the time spent and every scope narrowed or run skipped for budget, so a reader knows what the evidence does not cover.

In editing mode, commit deletions, consolidations, strengthenings, and gap tests separately, each as soon as it is verified, so an interrupted run loses little; they are different reviews.
Change only tests and source comments; never modify non-test source otherwise.

In review mode, post one review: inline comments on the affected tests, with the tables in `<details>` blocks.
Deletions, consolidations, strengthenings, gap tests, and small non-test changes that enable an independent oracle become requests to the author, with the current caught set as the bar.
