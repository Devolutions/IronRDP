---
name: new-crate
description: Create and integrate a new Rust crate in the IronRDP workspace. Use whenever adding, scaffolding, or initializing a crate under crates/, including requests that mention cargo new, a new library package, or registering a crate in the architecture.
---

# New crate

Run `cargo new --lib --vcs none crates/<crate-name>` from the repository root.
Do not hand-create the directory or initial `Cargo.toml`.

In the generated manifest:

- Preserve the workspace inheritance entries.
- Set `version = "0.0.0"` and `publish = false`.
- Add `readme`, `description`, the repository's current `rust-version`, and `[lints] workspace = true`.
- Add:

  ```toml
  [lib]
  doctest = false
  test = false
  ```

Invoke `crate-readme-writer` to write a short `README.md`.
Add the crate to the appropriate `ARCHITECTURE.md` tier by following its existing entry pattern.
Invoke `public-dependency` whenever adding a dependency.
