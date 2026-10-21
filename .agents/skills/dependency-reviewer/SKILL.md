---
name: dependency-reviewer
description: Review the dependencies a diff or pull request adds or widens against the `dependency-pruner` rules - load-bearing, no proc-macro in a foundational package, plain code over a package unless unsafe, minimal and maintained alternatives - and flag what is probably not necessary. Use when a diff changes a manifest or lockfile, or when asked whether a dependency belongs.
---

# Dependency reviewer

Invoke `dependency-pruner` and use its decision rules as review criteria without running its procedure; this skill judges the manifest and lockfile changes and does not edit.
Review only dependencies the diff adds or widens, features and transitive additions included, unless the user asks for a whole-package audit.

For each direct addition, find the code that uses it and ask what removing it would take; for a transitive one, find the direct dependency or feature that pulls it in.
A few straightforward, testable lines of safe code is a finding, with the lines sketched, unless they would reimplement security-sensitive or standards-defined behaviour; a replacement needing `unsafe` is not.
A proc-macro added to a foundational package is a finding unless the pull request names the feature that cannot exist without it and the diff bears that out.
A package that duplicates one already in the tree, directly or transitively, is a finding naming the existing one.
A heavy package used for one small part is a finding naming the minimal alternative when one exists; absent one, say so rather than guess.
A dependency whose pruner report entry survives these questions is not a finding; one with no entry is, because the author's verification is the evidence that it is load-bearing.
Report each finding with the manifest line or the lockfile entry and its parent edge, what the dependency does in the diff, and the replacement or the evidence that would justify keeping it; return no findings when every addition is load-bearing.
