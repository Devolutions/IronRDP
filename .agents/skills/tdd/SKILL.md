---
name: tdd
description: Red-green loop for one behaviour at an agreed seam - failing test first, the least code that passes, refactoring outside the loop. Use for test-first bug fixes and small features, or whenever the user says TDD or red-green.
---

# Test-driven development

`test-writer` decides what a good test is and `test-oracle-reviewer` where its expected value may come from; this skill is the loop around them.

## Seam

Name the seam before the first test: the public boundary where the behaviour is observed, taken from the intent or contract file, the bug report, or the existing public interface.
Ask the user only when choosing between seams would create or change a design decision, saying what each catches and misses.
No test goes below the seam, because a test against internals passes on wrong behaviour and fails on a harmless refactor.

## Red

Write one test for one behaviour, run it, and read the failure: the assertion on the behaviour must be what fails.
A compile error, missing fixture, or setup panic is not red (unless compilation failure is the behaviour under test), since it goes away once that other problem is fixed and proves nothing.
For a bug, the expected value is one `test-oracle-reviewer` would class as independent, never the output of the fixed code, which only shows the fix did what it did.
A behaviour that needs a decision no intent, contract, or report states stops the loop: ask, or run `grill-intent`, before writing the assertion, because the test becomes the specification the code is bent to.

## Green

Write the least code that passes this test and nothing for the test you expect next; a plainly stubbed answer is the signal for the next test, not a reason to write more code now.
Run the one test while iterating and the unit's suite before each commit, so the loop leaves only green commits.

## One slice at a time

One test, one change, repeat; each test responds to what the last cycle showed.
Do not write all the tests first: tests written in bulk check the shape you imagined, go insensitive to what the code does, and fix the test structure before you understand the implementation.

## Refactor outside the loop

Refactoring is its own commit after green, suite green before and after; a design finding from implementation or `code-review` is acted on there, so a refactor never hides behind a behaviour change.
