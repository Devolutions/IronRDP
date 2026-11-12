---
name: crate-readme-writer
description: Write a Rust crate README as a concise description of its purpose, integration point, and public interface. Use when creating a crate README, documenting a planned crate, or substantially revising an existing crate's introduction.
---

Use `docs-writer` for the prose.
For a new crate, write the README before implementation so the proposed interface can be reviewed while changing it is cheap.
For an existing crate, verify the manifest and implementation before making behavior claims.

Explain why the crate belongs in the repository, its principal capability, expected integration point, and platform or architectural constraints.
Use a minimal example only when it clarifies the interface better than prose.
A long or complicated introduction is a reason to reconsider the crate's scope rather than expand it into an exhaustive specification.
After implementation, verify claims and examples against the code.
