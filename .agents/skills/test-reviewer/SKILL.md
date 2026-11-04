---
name: test-reviewer
description: Review changed Rust or C# tests against the `test-writer` rules, including a named bug, an independent oracle, non-branching parametrized bodies, checked snapshots, and real properties. Use when a diff or pull request adds or changes tests, or when asked to review test organization or parametrization.
---

Invoke `test-writer` and use its rules as review criteria without following its generation workflow.
Review only changed tests unless the user requests a broader audit.
Run `test-oracle-reviewer` on each changed test; this skill covers a test's shape, not where its expected values come from.
When a test's name and assertions do not say which bug it catches, ask the author rather than guess.
When the change touches an invariant from an applicable intent or contract file that no test checks, ask what pins it and whether a property test could exercise it across generated inputs.
A new or changed snapshot is a behavior claim; check that the pull request intends it and that the value is right.
Report each violation with its exact location and a concrete correction, and group repeated instances that share one correction.
Return no findings when the changed tests already comply.
