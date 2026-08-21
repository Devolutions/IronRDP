---
name: commit-scope
description: Derive and validate the canonical scope for IronRDP Conventional Commit and pull-request titles. Use whenever composing, reviewing, correcting, or checking a commit or PR title, especially when several crates or product surfaces intersect.
---

# Commit scope

Choose at most one optional scope for `<type>[optional scope][!]: <description>`.

## Canonical scopes

```text
meta core error sequence pdu str bulk graphics config input connector session driver
svc dvc cliprdr rdpdr rdpsnd rdpeai displaycontrol rdpei echo egfx rdpeudp rdpeusb rdpemt rdcleanpath
tls mstsgu vmconnect

client viewer agent daemon rpc activex server web ffi replay

xtask fuzz release pr-automation agents
```

Most scopes match an `ironrdp-*` component after removing the prefix.
Use these aggregate mappings:

- `meta`: the `ironrdp` meta crate.
- `graphics`: `ironrdp-graphics` and `ironrdp-nscodec`.
- `config`: `ironrdp-cfg`, `ironrdp-propertyset`, and `ironrdp-rdpfile`.
- `input`: `ironrdp-input` and `ironrdp-ainput`.
- `driver`: `ironrdp-async`, `ironrdp-blocking`, `ironrdp-tokio`, and `ironrdp-futures`.
- `dvc`: DRDYNVC infrastructure, the COM plugin, and the pipe proxy, but not specific dynamic channels.
- `cliprdr`, `rdpdr`, and `rdpsnd`: each protocol and its format or native support crates.
- `rdpeai`: MS-RDPEAI audio input protocol and related client capture wiring.
- `server`: `ironrdp-acceptor` and `ironrdp-server`.
- `web`: WASM bindings, the Rust web helper, and everything under `web-client`.
- `ffi`: the Rust FFI and generated or manual .NET bindings, but not ActiveX.
- `replay`: `ironrdp-replay-client`.
- `fuzz`: fuzz targets, harnesses, corpora, and fuzzing automation.
- `release`: changelog, packaging, publishing, and release automation.
- `pr-automation`: automated PR classification and review infrastructure.
- `agents`: agent instructions and reusable skills.

## Selection rules

1. Scope the contract or behavior being changed, not every touched path.
2. When another component owns the change, ignore supporting tests, documentation, generated files, manifests, lockfiles, and call-site adaptations.
3. Prefer the component defining the behavior over components that merely consume it.
4. Split independent contract changes when practical.
5. Omit the scope when an indivisible change has multiple equal owners.
6. Never combine scopes or invent a catch-all scope; explain secondary effects in the body.
7. Add a new scope only for a distinct contract that does not fit an existing aggregate.

Use the component scope with any applicable type:

```text
fix(cliprdr): reject invalid file ranges
feat(driver): expose multitransport handoff
ci(release): sign binary artifacts
build(web): update frontend dependencies
```

Do not use types or cross-cutting aspects as scopes.
This excludes `ci`, `test`, `docs`, `build`, `deps`, `perf`, `tooling`, `automation`, `workspace`, `protocol`, `channels`, `native`, `security`, and `api`.
Use no scope for genuinely repository-wide changes.

## Tests

Use `test` by default.
Use only `test(core)` for core-tier tests, generators, fuzzing, and fixtures, or `test(extra)` for extra-tier integration infrastructure.
Do not use component scopes with `test`.
A production change accompanied by tests keeps its production type and scope.

Outside the `test` type, `core` means `ironrdp-core`.
`extra` is valid only in `test(extra)`.
