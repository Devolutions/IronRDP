---
name: new-crate
description: Create and integrate a new Rust crate in the IronRDP workspace. Use whenever adding, scaffolding, or initializing a crate under crates/, including requests that mention cargo new, a new library package, or registering a crate in the architecture.
---

# New crate

Inspect the root `Cargo.toml`, nearby same-tier crates, and `ARCHITECTURE.md` before creating files.
Determine the crate's name, purpose, architectural tier, API-boundary status, and platform or `no_std` constraints from the request and repository rules.

1. Run `cargo new --lib --vcs none crates/<crate-name>` from the repository root.
   Do not hand-create the directory or initial `Cargo.toml`.
   Preserve the workspace inheritance entries generated from `[workspace.package]`.
2. Align `[package]` with a nearby same-tier crate.
   Add `readme`, `description`, and the repository's current `rust-version`.
   Add `[lints] workspace = true`.
3. Add these target settings:

   ```toml
   [lib]
   doctest = false
   test = false
   ```

4. Remove Cargo's sample function and test before adding the crate's real implementation.
5. Invoke `crate-readme-writer` and write a short `README.md` before implementing crate behavior.
6. Add the crate to the appropriate tier in `ARCHITECTURE.md`.
   Follow the existing `#### [`crates/<crate-name>`](./crates/<crate-name>)` heading and concise-description pattern.
   For a Community Tier crate, append the responsible maintainer suffix used by neighboring entries.
   State API-boundary or architectural invariants only when they materially apply.
   Do not reorder or rewrite unrelated entries.
7. Add dependencies and implementation only after the README and architecture entry establish the crate's role.
