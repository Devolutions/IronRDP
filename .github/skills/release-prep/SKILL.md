---
name: release-prep
description: Reviews and finalizes a release-plz "chore(release): prepare for publishing" PR for the IronRDP workspace. Use this skill whenever the user asks to review, fix, prepare, or finalize a release PR; mentions release-plz, version bumps, CHANGELOG cleanup, or "publishing crates"; or references a PR titled "chore(release): prepare for publishing". Invoke proactively any time work involves auditing or editing CHANGELOG.md / Cargo.toml version fields across multiple crates ahead of publishing to crates.io.
---

# IronRDP release-plz PR review & finalization

release-plz does most of the mechanical work (computing bumps, generating changelog entries from conventional commits, updating dependency version requirements, refreshing `Cargo.lock`). This skill covers the **human-in-the-loop pass** that release-plz cannot do automatically: verifying bumps against the public-dependency rule, rewriting per-crate changelog entries, and deduplicating commits that collapse into a single net change.

All fixes go on the **release-plz PR branch** directly. Do not rewrite history on `main` for cosmetic changelog reasons; if real source/doc changes are needed, land them on `main` first and let release-plz regenerate the PR.

## The public-dependency propagation rule

A dependency line in `Cargo.toml` annotated with `# public` means that the dependency's types appear in this crate's *public* API surface.

- **Breaking change in a `# public` dependency ⇒ breaking change in the depending crate** (minor bump pre-1.0, major bump post-1.0). The downstream changelog must reflect that it is breaking, even if the downstream's own code changes were trivial recompiles.
- A *patch-level* bump in a `# public` dependency does **not** force any bump in downstream crates: workspace `Cargo.toml` files use `"0.7"`-style major.minor requirements only, so Cargo's resolver picks the new patch up automatically.
- A dependency *without* the `# public` marker is internal. A breaking change in it is just a patch for the depending crate (assuming the public API is unchanged).

Enumerate public-dep edges with `grep -n '# public' crates/*/Cargo.toml`. The dep's bump kind is visible in the release-plz PR body's "New release" summary.

For each crate in the release, cross-check the proposed bump against (a) its own commits' conventional-commit signals (`feat!`, `BREAKING CHANGE` footer, etc.) and (b) the propagation rule above. When the bump is wrong, fix it on the PR branch: edit `version = "..."` in that crate's `Cargo.toml`, cascade to the version requirement in any consumer crate's `Cargo.toml`, and update the version-header line at the top of the corresponding `CHANGELOG.md` (`## [[X.Y.Z](...)] - YYYY-MM-DD`, including the `vOLD...vNEW` compare URL).

## Changelog quality pass

release-plz pulls each commit's subject + body into the changelog of *every* crate touched by that commit. Many IronRDP commits span multiple crates with very different semantics on each side — e.g. a breaking change in `ironrdp-pdu` that requires a mechanical call-site update in `ironrdp-connector`. The PDU-flavored description does not belong in the connector's changelog.

**For each duplicated entry:**

1. **Default:** rewrite to describe what *this* crate actually changed, inferred from the diff for this crate in the originating commit. Use `git show <sha> -- crates/<this-crate>/` to see only this crate's slice.
2. **Fallback:** if the change here is genuinely just "recompile against new dep" with nothing more specific to say, replace the entry with a short note such as `Update <dep> dependency`. There *is* a real code change (release-plz wouldn't have inserted an entry otherwise — at minimum the dep version requirement moved), so the changelog should still acknowledge it; just don't parrot the upstream description.
3. Preserve the conventional-commit category mapping (Features / Bug Fixes / Documentation / etc.) defined in `cliff.toml`, but recategorize if the rewritten entry fits a different bucket better.
4. If the rewrite changes a non-breaking entry into a breaking one (or vice versa) because of the public-dep rule, add or remove the `[**breaking**]` prefix accordingly, and revert the breaking bump which was wrongly applied (minor bump pre-1.0, major bump post-1.0).

### Deduplication across the release window

Multiple commits in the release window often collapse into a single net change. Spend real effort on this — a clean changelog is one of the highest-leverage outputs of this pass. Common patterns:

- **Revert before release:** both entries should be dropped (or, if the revert was partial, replaced with a single entry describing the final state).
- **Iteration on an unreleased feature:** initial commit introduces feature X, follow-ups tweak its API or behaviour, all within the same release window. Collapse into a single entry describing X *as it actually ships*.
- **Bug fix on an unreleased feature:** if the bug only ever existed in the unreleased code, fold the fix into the feature entry.
- **Mechanical follow-ups** (review feedback, clippy fixes on a feature PR): keep only the feature entry.

When merging entries, preserve every relevant `[#NNNN]`/commit-SHA link so the historical trail stays navigable. The headline text should describe the *final* outcome, not the journey.

## Scope of edits

Only two file kinds should be touched on the release-plz PR branch:

- `crates/<crate>/CHANGELOG.md` — content rewrites, drops, breaking-tag fixes, version-header bumps.
- `crates/<crate>/Cargo.toml` — `version = "..."` corrections and cascading version-requirement updates in consumer crates.

Do **not** touch source code, READMEs, MSRV, `rust-toolchain.toml`, or CI files here. Those belong on `main`.

## Procedure

1. Inspect the PR's changes from the working tree: `git --no-pager diff origin/main -- '**/CHANGELOG.md' '**/Cargo.toml'` for the substantive deltas, and read the PR body for the "New release" bump table (use `gh pr view` only if the body isn't already provided in context).
2. Read the PR body's "New release" bump table.
3. Enumerate `# public` dep edges and cross-check every proposed bump against the propagation rule and the originating commits' conventional-commit signals. Note mismatches.
4. For each crate's `CHANGELOG.md` section in the diff, classify entries as *own change*, *public-dep ripple*, or *internal-dep ripple / unrelated*. Rewrite or replace per the rules above. Then deduplicate across the release window.
5. Apply any `Cargo.toml` version corrections from step 3, cascading to consumer crates' version requirements and the corresponding changelog version headers.
6. If you edited any `Cargo.toml` version, run `cargo check --workspace` (without `--locked`, so `Cargo.lock` can be regenerated) and commit the resulting lock delta. Then run `cargo xtask check locks -v`, which verifies no lock file is left uncommitted.
7. Push the fixups with conventional-commit-shaped messages (typically `chore(release): ...`) so they don't pollute the next release window.
8. Summarize the changes and any judgement calls — especially dropped entries and escalated bumps — for the human reviewer.
