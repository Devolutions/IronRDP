---
name: crate-readme-writer
description: Write or substantially revise a Rust crate's README.md as a concise, user-facing design document. Use whenever creating a crate README, documenting a planned crate, or defining a crate's purpose and public interface before implementation.
---

# Crate README writer

Invoke `docs-writer` and apply its rules throughout.

For a new crate, write the README before implementing behavior.
Use it as a lightweight specification: changing prose and examples is cheaper than changing code, and a written proposal gives collaborators a concrete design to review.
For an existing crate, inspect its manifest and implementation first so the README describes current behavior rather than an imagined interface.

1. Identify the intended reader, the problem the crate solves, and why it belongs in the repository.
2. Lead with the crate name and a one- or two-sentence description of its purpose.
3. Describe the smallest useful public contract: principal capability, expected integration point, and important platform or architectural constraints.
4. Add a minimal usage example only when it clarifies the intended API better than prose.
5. Revise the proposed interface in the README until it is coherent before committing to implementation details.
6. Treat a long or complicated README as design feedback: narrow or split the crate instead of writing an exhaustive specification.
7. Link to existing project or protocol documentation rather than duplicating it.
8. After implementation, verify every claim and example against the code while preserving the README's concise introductory role.

Write `README.md` directly.
Keep it short enough to expose unclear scope and unnecessary complexity.

This workflow follows [Readme Driven Development](https://tom.preston-werner.com/2010/08/23/readme-driven-development).
