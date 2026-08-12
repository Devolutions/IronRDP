---
name: public-dependency
description: Add or change Rust crate dependencies while preserving IronRDP's public-dependency markers. Use whenever adding, updating, renaming, removing, or changing features of a Cargo dependency, or when a public API starts or stops exposing dependency-owned types or traits.
---

# Public dependency

Use `cargo add --package <crate> <dependency>` and its flags, then append `# public` on the same manifest line when the dependency is exposed by the crate's public API:

```toml
dependency = "1" # public
```

Exposure includes dependency-owned types or traits in re-exports, signatures, public fields, aliases, implementations or bounds, and associated types.
Whether the dependency is direct, optional, or workspace-internal is irrelevant.
Do not mark implementation-only, test, or example dependencies.

The marker records that a breaking dependency change can require a breaking crate release.
Add or remove it as API exposure changes, and preserve a short condition or type note when it clarifies non-obvious exposure.
