---
name: release-prep
description: Review and correct an IronRDP release-plz release PR before publishing. Use when auditing version bumps, public-dependency propagation, or generated per-crate changelogs in a prepare-for-publishing PR.
---

Apply fixes on the release-plz PR branch.
Source, README, MSRV, toolchain, and CI changes belong on the repository's default branch first so release-plz can regenerate the release PR.
Resolve the PR's actual base branch rather than assuming `main` or `master`.

Use `public-dependency` to identify `# public` edges, then compare each proposed bump with the crate's own changes and the PR's new-release table.
A breaking change in a public dependency requires a breaking release of its consumer: minor before 1.0, major after 1.0.
A patch dependency release needs no consumer bump when the existing requirement already accepts it.
An internal dependency's breaking release needs only a patch consumer release when the consumer's public API is unchanged.
Correct crate versions, consumer requirements, changelog version headings, and compare URLs together.

For each new changelog entry, inspect the originating commit's diff for that crate.
Describe that crate's user-visible change; use a short dependency-update note when it only adapts to another crate.
Do not copy an upstream feature or breaking-change description into a mechanically updated consumer.
Apply `[**breaking**]` according to the consumer's public API and public-dependency rule.

Collapse changes that cancel or refine one unreleased behavior: remove full revert pairs, describe a feature as it ships, and fold its unreleased bug fixes and mechanical follow-ups into that entry.
Preserve relevant PR and commit links when combining entries.

Preserve `cliff.toml`'s generated format: section order and `<!-- N -->` prefixes, version headings, blank lines, two-space body indentation, `[**breaking**]`, and link destinations.
Move a rewritten entry to its correct existing category when necessary.
The `[#1234](...)` and `[abcdef0123](...)` forms are inline links; preserve them as generated.

Limit release-branch edits to `crates/*/CHANGELOG.md`, crate versions and dependent requirements in `Cargo.toml`, and the generated `Cargo.lock`.
After manifest changes, run `cargo check --workspace` without `--locked` to regenerate the lockfile.
Commit the resulting release changes before running `cargo xtask check locks -v`, which checks for uncommitted lockfiles.
Use `commit-and-pr-conventions` for authorized commits and pushes, and report dropped entries and changed bump decisions for review.
