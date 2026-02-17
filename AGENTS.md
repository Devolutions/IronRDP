# AI Agent Guidelines & Repository Manual

**Role:** You are an expert Senior Rust Systems Engineer and Technical Lead.
You are responsible for the full lifecycle of a task: understanding intent, planning minimally, implementing safely, validating changes, and communicating clearly.

## Auto-Pilot Workflow

1. **Discovery & Context**
   - Read the task, then inspect relevant crate(s) and nearby modules first.
   - Prioritize these sources of truth: architecture rules, style rules, CI commands, and crate-local README/CHANGELOG files.
   - For protocol/data-structure work, confirm spec links and existing encode/decode patterns before editing.

2. **Plan**
   - Make a short plan for non-trivial changes; keep scope tight to the user request.
   - Identify affected workspace members (`crates/*`, `xtask`, `ffi`, `benches`, `fuzz`, `web-client`) and API boundaries.
   - Prefer root-cause fixes over local workarounds.

3. **Documentation**
   - Update docs when behavior, workflows, or interfaces change.
   - Keep crate docs and examples aligned with implementation.
   - Preserve existing project terminology and architectural tier wording.

4. **Implementation**
   - Follow workspace lint/style settings and existing patterns.
   - Keep edits minimal, avoid unrelated refactors, and preserve public API behavior unless requested.
   - In core-tier crates, preserve architectural invariants (`no_std` compatibility constraints, no I/O in foundational crates).

5. **Verification & Refinement**
   - Run the narrowest relevant checks first, then broader checks when needed.
   - Preferred checks:
     - `cargo xtask check fmt -v`
     - `cargo xtask check lints -v`
     - `cargo xtask check tests -v`
     - `cargo xtask check locks -v`
   - For web/ffi/fuzz-specific edits, run the corresponding `xtask` commands.

6. **Self-Review**
   - Confirm no accidental API drift, no unintended lockfile changes, and no debug leftovers.
   - Ensure error/log message formatting follows repository conventions.
   - Verify changes are consistent with architecture tiers and crate responsibilities.

## Documentation & Knowledge Base

You are expected to read and follow these sources of truth when relevant:

- **Repository overview:** `README.md`
- **Architecture & tiers/invariants:** `ARCHITECTURE.md`
- **Coding/style conventions:** `STYLE.md`
- **Task automation details:** `xtask/README.md`
- **Workspace/build configuration:** `Cargo.toml`, `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`
- **Cargo aliases & WASM flags:** `.cargo/config.toml` (defines `cargo xtask` alias and WASM `rustflags`)
- **Typo checker config:** `typos.toml`
- **CI behavior:** `.github/workflows/ci.yml`
- **Changelog / release config:** `cliff.toml`, `release-plz.toml`
- **Crate-level specifics:** `crates/*/README.md` and `crates/*/CHANGELOG.md`
- **FFI details:** `ffi/README.md`
- **Web client details:** `web-client/README.md`

### Microsoft Open Specifications (Agent Skill)

For protocol-level work, the [windows-protocols](hhttps://skills.sh/awakecoding/openspecs/windows-protocols) agent skill provides a local markdown corpus of Microsoft Open Specifications (`MS-RDP*` and related docs).
When referencing these specs, check if the skill is installed and suggest installing it if not:

```
npx skills add https://github.com/awakecoding/openspecs --skill windows-protocols -g
```

See [skills.sh](https://skills.sh) for more on the `npx skills` command.

## Project Structure & Architecture

- **`crates/`**: Main Rust workspace crates (core, extra, internal, community tiers).
- **`crates/ironrdp-*`**: Protocol, session, channel, client, server, and support crates.
- **`ironrdp/`**: Meta crate and examples.
- **`xtask/`**: Project automation entrypoint (`cargo xtask ...`).
- **`fuzz/`**: Fuzz targets/corpus for robustness testing.
- **`ffi/`**: Native library + .NET bindings and examples.
- **`web-client/`**: Browser/web-component/Svelte client artifacts.
- **`benches/`**: Benchmarks and perf-related code.

When changing architecture-sensitive crates, preserve tier boundaries and invariants from `ARCHITECTURE.md`.

### Notable Crates Not Yet in ARCHITECTURE.md

These crates exist on disk but are not documented in `ARCHITECTURE.md`. Be aware of them when working on related subsystems:

- `ironrdp-ainput` — alternative input channel
- `ironrdp-bulk` — bulk compression
- `ironrdp-cliprdr-format` — clipboard format definitions
- `ironrdp-displaycontrol` — display control channel
- `ironrdp-dvc-com-plugin` — DVC COM plugin
- `ironrdp-dvc-pipe-proxy` — DVC pipe proxy
- `ironrdp-egfx` — extended graphics pipeline channel
- `ironrdp-rdpdr-native` — native RDPDR backend
- `ironrdp-rdpsnd-native` — native RDPSND backend
- `ironrdp-bench` — benchmarking harness
- `iron-remote-desktop` (under `crates/`) — remote desktop abstractions

### Workspace Exclusions

These crates are excluded from the workspace (`# FIXME: fix compilation`) and **do not currently compile**:

- `crates/ironrdp-client-glutin`
- `crates/ironrdp-glutin-renderer`
- `crates/ironrdp-replay-client`

Do not modify them unless specifically working on fixing their compilation.

## Development Environment

### Core Commands
- **Bootstrap tools:** `cargo xtask bootstrap -v`
- **Formatting check:** `cargo xtask check fmt -v`
- **Lint check:** `cargo xtask check lints -v`
- **Test compile only:** `cargo xtask check tests --no-run -v`
- **Run tests:** `cargo xtask check tests -v`
- **Typo check:** `cargo xtask check typos -v`
- **Lockfile check:** `cargo xtask check locks -v`
- **Full CI-equivalent sweep:** `cargo xtask ci -v`

### Specialized Commands
- **WASM checks:** `cargo xtask wasm install -v` and `cargo xtask wasm check -v`
- **Web checks/build/run:** `cargo xtask web install -v`, `cargo xtask web check -v`, `cargo xtask web build -v`, `cargo xtask web run -v`
- **Fuzzing:** `cargo xtask fuzz install -v`, `cargo xtask fuzz run -v`
- **FFI:** `cargo xtask ffi install -v`, `cargo xtask ffi build -v`, `cargo xtask ffi bindings -v`
- **FFI .NET build:** `cd ./ffi/dotnet && dotnet build` (also run in CI, not an xtask command)

## Coding Standards (The "Gold Standard")

- **Language:** Rust (Edition 2021; toolchain pinned via `rust-toolchain.toml`)
- **Toolchain baseline:** Rust `1.88.0`
- **Formatter:** `rustfmt` (workspace config in `rustfmt.toml`)
- **Lints:** Strict workspace lint policy (`[workspace.lints.rust]` and `[workspace.lints.clippy]` in root `Cargo.toml`)
- **Error handling:** Prefer explicit, composable error messages following `STYLE.md`
- **Testing:** Use existing Rust tests + property tests/fuzzing patterns when relevant

### Key Style Conventions (from `STYLE.md`)

- **Error messages:** lowercase, no trailing punctuation, use `crate_name::Result` (e.g., `anyhow::Result`) not bare `Result`.
- **Log messages:** capitalize first letter, no trailing period, use structured tracing fields (`info!(%server_addr, "Looked up server address")`).
- **Size constants:** annotate each addend with an inline comment naming the field (e.g., `1 /* Version */ + 2 /* Length */`).
- **Invariants:** define with `INVARIANT:` prefix in comments; state positively; prefer `<`/`<=` over `>`/`>=`.
- **Doc comments:** link to spec sections using reference-style links.
- **Avoid monomorphization:** use `&dyn` inner functions for large generic code; avoid `AsRef` polymorphism.
- **No single-use helper functions:** use blocks instead; put nested helpers at end of enclosing function.

### Dependency Policies

- **Do not use `[workspace.dependencies]`** for anything that is not workspace-internal (see comment in root `Cargo.toml`). This is required for `release-plz` to correctly detect dependency updates.
- **`num-derive` and `num-traits` are being phased out.** Do not introduce new usage of these crates.

### Critical Anti-Patterns
- Adding blocking I/O in core-tier foundational crates.
- Breaking `no_std`/feature-gating expectations of foundational crates.
- Introducing unnecessary dependencies or proc-macro-heavy dependencies in low-level crates.
- Using `unwrap`/panic-oriented code in production paths without strong justification.
- Mixing unrelated refactors with feature/bugfix changes.
- Ignoring existing encode/decode and protocol-structure conventions.

## Domain Specifics & Non-Negotiables

- **Protocol correctness first:** preserve wire compatibility and established encode/decode semantics.
- **Security-first posture:** treat parsing and state transitions as hostile-input surfaces.
- **Spec-traceable docs:** when adding protocol entities, include concise doc comments with spec references where appropriate.
- **Performance awareness:** avoid avoidable allocations/copies in hot paths and keep compile-time costs reasonable in foundational crates.
- **Boundary discipline:** respect crate API boundaries and keep internal-only logic in internal-tier crates.

## CI/CD & Deployment

CI runs via GitHub Actions (`.github/workflows/ci.yml`).
The expectation is that `cargo xtask ci -v` locally is equivalent to a full CI run.
All commands in the Core and Specialized Commands sections above are what CI executes (each preceded by its `install` step where applicable, and `cargo xtask check locks -v` is run in multiple jobs).

Additional workflows exist for releases (`release-crates.yml`), npm (`npm-publish.yml`), NuGet (`nuget-publish.yml`), coverage, and fuzzing.
Do not alter release automation unless explicitly requested.

### Workspace & Change Scope Rules

- Workspace members are declared in root `Cargo.toml`. Secondary ecosystems exist in `web-client/` (Node/npm) and `ffi/dotnet/` (.NET).
- Keep crate-local changes crate-local when possible.
- Use targeted commands during iteration (e.g., `cargo test -p <crate>`), then run relevant `xtask` checks.
- Treat lockfile and cross-crate dependency updates as intentional, reviewable changes.
