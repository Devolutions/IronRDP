# IronRDP

Reusable agent skills live in `.agents/skills/`; use the matching skill before committing, opening a pull request, posting a GitHub comment, or writing prose.
Skills listed in `.github/skills-sync.yml` are overwritten weekly from `Devolutions/devolutions-gateway`; change those upstream, and keep repository-local skills out of that list.

Start a work session with `/contribute`, which picks the delivery flow by scale and the skills each step needs.

## Intent files

`INTENT.md` and `*.intent.md` files contain human-owned intent for the software.
Read all applicable intent files before modifying code.
Do not create, modify, or delete intent files.
If a requested change conflicts with intent, stop and surface the conflict for human resolution.

## Writing for the repository

Committed text (code, comments, docs, commit messages) and PR titles and descriptions never refer to work stages, decision identifiers, agent roles, sessions or planning documents.
State rationale, not history.

## Generated changelogs

Do not create, edit, or update `crates/*/CHANGELOG.md` during feature or fix work.
Crate changelogs are generated and updated exclusively in release-plz release PRs.
Crate changelogs are not normal hand-maintained documentation.

## Rust

`cargo xtask ci -v` runs the CI checks locally.
Think before silencing a lint, then use `#[expect(clippy::<lint>, reason = "...")]` so the suppression explains itself and expires when it stops being needed.
Write Rust with the `rust-style` skill and tests with the `test-writer` skill.
Most tests belong in `ironrdp-testsuite-core` and `ironrdp-testsuite-extra`, not inline in the crate; see `ARCHITECTURE.md` § Testing.
When decoding PDUs, follow the error-construction and unknown-value sections of `STYLE.md`.
Preserve the core-tier invariants listed in `ARCHITECTURE.md`; no tool checks them.

## Untested-property comments

When code must keep a property that no test checks, mark it with an `UNTESTED:` comment at that code, because passing tests prove nothing about it.
The comment states the property, why it matters, and what to check when changing the code.
List `UNTESTED:` properties in a PR comment as well, not in the PR description, because the description becomes the squash commit body.

## Protocol specifications

For Microsoft Open Specifications (`MS-RDP*` and related documents), use the [windows-protocols](https://skills.sh/awakecoding/openspecs/windows-protocols) skill, installed with `npx skills add https://github.com/awakecoding/openspecs --skill windows-protocols -g`.
