---
name: commit-and-pr-conventions
description: Follow the adopted commit and pull-request conventions. Use whenever creating a commit or opening, editing, reviewing, or preparing to squash-merge a PR, including requests to commit, push, or open a PR.
---

# Commit and PR conventions

Treat the PR title and initial body as the exact commit message that squash-and-merge will land.

## Commit message

- Read repository instructions first. Derive scopes with the repository-local `commit-scope` skill. If unavailable, use explicit repository rules or omit the scope rather than inventing one.
- Follow Conventional Commits: `<type>[optional scope][!]: <description>`.
- Target 50 characters and describe the resulting behavior. Do not drop repository-required scopes to meet that target; when they make it impossible, keep the remaining description as concise as practical.
- Separate subject, body, and footers with blank lines.
- For standalone commits, wrap body lines at 72 characters. Explain why the change was needed, how it solves it, and material side effects.
- Put an existing ticket key or link in a footer, not the subject: `Issue: PROJECT-123`. Follow repository-specific footer format when an issue exists. When none exists, omit the footer rather than fabricating `Issue: N/A`.
- Add applicable Conventional Commit footers and `Co-authored-by: Name <email>` trailers for collaborators.
- Use `fix` for bugs, `feat` for features, `build` for build or dependencies, `chore` for non-product tools or configuration, `ci` for automation, `docs` for documentation only, `style` for non-semantic edits, `refactor` for restructuring, `test` for tests, and `perf` for performance.
- Keep every non-squashed development commit coherent and conventional.

```text
fix(storage): preserve collated indexes

Normalize server-expanded collations before comparing index definitions.

Issue: PROJECT-123
```

## Pull request

- Set the title to the intended squash commit subject exactly.
- Set the initial body to the intended squash commit body and footers exactly. Exclude PR-only preambles, checklists, and implementation journals.
- Do not manually wrap the PR body; GitHub handles display wrapping.
- Keep permanent history concise: record motivation, material behavior, and side effects.
- Put experiments, alternatives, extensive validation, and implementation chronology in follow-up comments.
- Update the title and initial body as the implementation evolves.
- Before opening, updating, or merging, read `title + blank line + body` as one commit message and correct any mismatch.
