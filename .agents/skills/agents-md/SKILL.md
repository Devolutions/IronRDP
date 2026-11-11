---
name: agents-md
description: Discipline for writing and updating AGENTS.md (or CLAUDE.md) so it holds only what an agent cannot infer - policy, gotchas, and pointers - and stays under about 40 lines. Use instead of /init when creating, editing, or adapting one.
---

`AGENTS.md` is loaded on every task, so every line is a cost paid everywhere.
Measured on real repositories (arXiv 2602.11988): developer-written files raised resolve rate about 4% and cost about 20%; LLM-generated files lowered resolve rate; repository overviews did not help.
What helped agents was locating the right place to change, not writing better patches.

## The test

A line earns its place when both hold:

> Would it cost more than one cheap feedback round to discover and fix?
> Does no skill or always-read file already state it?

A lint or formatting miss costs one run of CI or of the repository's check command and is not worth a line; a testsuite written against the library instead of the CLI is a pile to redo, and is.
Put positively: there is no way the agent learns this unless we write it.

## What belongs, in order of value

- A pointer to a non-standard layout: "Integration tests are centralized in `testsuite`; see `testsuite/AGENTS.md`."
- The default feedback loop when it is not the ecosystem's default: "`cargo xtask check-uncommitted` verifies local modifications."
- A policy with its reason: "Use `#[expect(lint, reason = "...")]` so the suppression explains itself and expires when it stops being needed."
- Repository rules no tool enforces: intent-file handling, writing conventions for committed text, comment markers such as `INVARIANT:` and `UNTESTED:`.
- The one fact a shared skill leaves to the repository, such as the issue-reference footer format, when it fits in a line; otherwise a repo-local skill.
- One opener saying where skills live, for agents that do not discover them.

What `/init` generates is the blacklist: product overview, directory map, command lists, "follow standard conventions", inventories of packages or scopes, and lossy summaries of `STYLE.md`.
Examples without the rule behind them are the costliest lines; agents copy the example, mistakes included.

Omit an opening banner that only repeats the repository, path, or body context, because it adds no guidance.
Keep headings that organize the rules or add context.

## Division of labour

```text
AGENTS.md    always loaded     policy, gotchas, pointers
skill        loaded on match   rules for one task; may load the skills it builds on
STYLE.md     loaded by a reviewer skill when a rule's intent is unclear
```

The root file enters every context, so about 40 lines is the rule of thumb; a nested file loads for fewer tasks and may run longer; past the mark, flag it for the human rather than cut a gotcha to fit.
Load each file at the latest point where it pays for itself; an always-loaded file pointing at a 700-line guide is a cost on every task, while a skill pointing at the one it depends on keeps both small.
A directory with its own policy gets a nested `AGENTS.md` and one pointer from the root.
Every `AGENTS.md` gets a sibling `CLAUDE.md` symlink to it, because Claude Code reads only `CLAUDE.md`.

## Keeping it true

- Verify every surviving sentence against the tree and the remote default branch; `/init` files go stale within weeks.
- When a line documents a workaround, fix the code instead; the file is a list of smells not yet fixed.
- When adapting another repository's file, report deviations with reasons instead of copying: a line true there may be false here.

`references/before-after.md` holds one 151-line file reduced to 27, with the reason each block was cut or kept and the maintainer's verbatim pushbacks; read it when unsure what counts as discoverable.
