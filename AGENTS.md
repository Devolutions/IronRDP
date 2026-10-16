# IronRDP

Reusable agent skills live in `.agents/skills/`; use the matching skill before committing, opening a pull request, posting a GitHub comment, or writing prose.

## Intent files

`INTENT.md` and `*.intent.md` files contain human-owned intent for the software.
Read all applicable intent files before modifying code.
Do not create, modify, or delete intent files.
If a requested change conflicts with intent, stop and surface the conflict for human resolution.

## Writing for the repository

Committed text (code, comments, docs, commit messages) and PR titles and descriptions never refer to work stages, decision identifiers, agent roles, sessions or planning documents.
State rationale, not history.

Code comments and doc comments follow the same rule as Markdown: each sentence starts on its own line and wraps only where it would exceed the line-length limit.

## Rust

`cargo xtask ci -v` runs the CI checks locally, on the toolchain pinned by `rust-toolchain.toml`.
Think before silencing a lint, then use `#[expect(clippy::<lint>, reason = "...")]` so the suppression explains itself and expires when it stops being needed.
Write Rust with the `rust-style` skill; the `code-review` skill runs `rust-style-reviewer` on changed Rust.
When decoding PDUs, follow the error-construction and unknown-value sections of `STYLE.md`.
Place `#[cfg(test)]` modules at the end of their file or module, after all production items.

Preserve the tier invariants in `ARCHITECTURE.md`: core-tier crates do no I/O, stay `no_std`-compatible, and take no proc-macro dependencies.
Treat parsing and state transitions as hostile input, and preserve wire compatibility.
Crates in the root `Cargo.toml` `exclude` list do not compile; leave them alone unless fixing them.

Use `[workspace.dependencies]` only for workspace-internal crates, because release-plz cannot otherwise detect which dependants need a version bump.
Do not add new uses of `num-derive` or `num-traits`; they are being phased out.
Do not change release automation unless asked.

## Untested-property comments

When code must keep a property that no test checks, mark it with an `UNTESTED:` comment at that code, because passing tests prove nothing about it.
The comment states the property, why it matters, and what to check when changing the code.
List `UNTESTED:` properties in a PR comment as well, not in the PR description, because the description becomes the squash commit body.

```rust
// UNTESTED: decryption is blinded with the OS generator.
// Blinding changes no output; it is the rsa crate's countermeasure against timing attacks on the private key.
// When changing this call, keep `decrypt_blinded` with the generator from `os_rng()`.
```

## Web client

`iron-remote-desktop` is protocol-agnostic.
A method belongs in its base API (`UserInteraction`, `Session`, `SessionBuilder`) only if the web component itself calls it or the feature applies to every remote protocol backend; protocol-specific features go through the extension mechanism.

## Protocol specifications

The [windows-protocols](https://skills.sh/awakecoding/openspecs/windows-protocols) skill provides a local corpus of the Microsoft Open Specifications (`MS-RDP*` and related documents).
If it is not installed, suggest `npx skills add https://github.com/awakecoding/openspecs --skill windows-protocols -g`.
