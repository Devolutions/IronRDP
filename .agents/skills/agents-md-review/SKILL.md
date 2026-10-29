---
name: agents-md-review
description: Audit an existing AGENTS.md or CLAUDE.md line by line for wrong, stale, discoverable, duplicated, or content-free guidance and propose the pruned file. Use when asked to review, prune, shrink, or clean up one.
---

Verdicts apply the test in `agents-md`: more than one cheap feedback round to discover and fix, and stated in no skill or always-read file.

## Verify, then classify

Review the file as given and state which revision.
Check each block against reality before judging it; one piece of evidence per verdict is enough.

- Examples and formats against the tool that consumes them (changelog generator, release tooling, CI workflows): **Wrong**.
  Reading the tool's configuration is enough; what it would render wrongly is wrong whether or not the bad output has shipped.
- Lists against the tree and the remote default branch: **Stale**.
- Claimed conventions against merged-PR history: **Never a convention** when nobody follows it and the maintainer confirms nobody decided it.
  A maintainer may be introducing the rule deliberately, and a human committer proves nothing about who decided it, so an unfollowed rule the maintainer has not ruled on goes on the human-decision list.
- Rules against the installed skills and always-read files: **Duplicated**.
- Everything else against the test above: **Discoverable**, **Content-free**, or **Plain command**.
- An always-loaded pointer to a long guide: **Misplaced**; it belongs in the skill that needs it.
- Opening banners against the file location and body: **Content-free** when they add no context, because the scope or purpose is already stated.
  Keep headings that organize the rules or add context.

A block can earn more than one verdict, each with its own evidence; lead with the one that needs no human confirmation, so pruning can start before the maintainer answers.

Also look for what is **Missing**: policy the organisation applies elsewhere (intent-file rules, writing conventions, comment markers), non-standard layouts with no pointer, and an `AGENTS.md` with no sibling `CLAUDE.md` symlink.
`agents-md/references/before-after.md` is one repository's worked answer; re-verify every row against the repository in front of you, never paste it.

## Report

```md
| Block | Verdict | Evidence |
|---|---|---|
| `Issue: #456`, `Issue: N/A` as valid footers | Wrong, Never a convention | `cliff.toml` turns every `Issue:` value into a Jira link; maintainer confirms neither was decided |
| Tests drive the CLI binary, not the library; see `testsuite/AGENTS.md` | Keep | redoing a library-level testsuite is more than one round; no skill states it |
```

Then propose the full pruned file, list what moved to a skill or a nested `AGENTS.md`, and list what needs a human decision.
Do not apply the changes unless asked.
