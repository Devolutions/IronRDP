---
name: dependency-pruner
description: Audit the dependencies a change added or widened before its pull request opens, keep each only where it is load-bearing, and replace the rest with a few lines, a lighter package, or one already in the tree. Use at the end of a change, as the last worker task before the pull request opens, or when asked to trim or justify a package's dependencies.
---

# Dependency pruner

A dependency is a maintenance contract: every one costs build time, audit surface, and a future upgrade, so each must be load-bearing, which means removing it would cost more than it saves.
Run once per change, on the diff, after the code is complete, because a package added mid-way is often not needed by the end.

## Inventory

List every direct dependency, feature, and version bump the manifest diff adds or widens, then every package the lockfile diff adds and the direct dependency or feature that pulls it in, since a feature flag or a transitive addition expands the build as much as a new direct entry.
For each direct one, find the code that uses it and write down what it does there and what removing it would take.

## Decide

- Replaceable in a few straightforward, testable lines of safe code: replace it, because the package costs every upgrade and the lines cost one review; except where the lines would reimplement security-sensitive or standards-defined behaviour - parsing, cryptography, encodings - which is where a short implementation is wrong in ways no test of yours catches.
- Replaceable only with `unsafe`: keep it, since `zerocopy`, `bytemuck`, and their kind are audited where a hand-written transmute is not.
- A proc-macro in a foundational package - one kept to few dependencies because many others build on it - goes unless the feature cannot exist without it, because a macro adds a host-side build and the common derive stacks bring `syn` into every dependent's build graph; an error type there is `Display` and `Error` written by hand, not `thiserror`.
- Already in the tree, directly or transitively: reuse it rather than add a second package for the same job.
- Needed but heavy: look for a minimal alternative that does only the needed part.
- Two candidates remain: take the one with the stronger maintenance evidence - issues and pull requests answered, releases when fixes were needed, more than one active maintainer - since the one that stops being maintained becomes your code.
- A dependency the applicable intent file allows or forbids by name follows the intent.

## Change

Remove what the decisions say to remove, then inspect the resolved graph and run the repository's dependency policy checks (for Cargo, `cargo tree -d` for duplicate versions and the feature graph for enabled edges), because a removal changes resolution and feature unification for packages you did not touch.
Commit the pruning apart from the feature work, so its review is about dependencies and nothing else.
Report every kept dependency with what it does and why replacing it costs more, every removed one with what replaced it, and the net change in direct and transitive counts.
