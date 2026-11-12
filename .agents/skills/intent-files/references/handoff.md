# Handoff note

Written with a set of intent drafts so the human can confirm the assembled file.
It is a session artifact: save it in the harness's artifact area when one exists, otherwise outside the source tree, and never stage or commit it.
When the drafts go up in a pull request, its content becomes a PR comment, not part of the body, because the body becomes the squash commit and the changelog.

```md
# Intent drafts: handoff

<Code revision the interview read.>

## Provenance

| # | Commitment as written | Human's words (if tidied) | Level |
|---|---|---|---|

## Left out on purpose

<What came up and went to AGENTS.md, a contract, another component's intent file (named), or the PR instead, so it is not asked again.>

## Placement

<Which file each commitment landed in, and which directories symlink to a shared part.>

## Pending experiments

<Questions sent to a `variants` run, each with its candidates; when a brief returns, only a sentence the human adopted or rewrote is recorded, with its provenance level.>
```

Levels:

| Level | Meaning | Where it goes |
|---|---|---|
| 1 | The human's own words, quoted verbatim | Intent text |
| 2 | The human's words tidied to the conventions, original quoted beside | Intent text |
| 3 | Agent wording the human adopted: an option label or candidate sentence they copied, or a delegated choice they approved | Intent text, marked so the record shows the human decided but did not word it |
| 4 | Filled in by the agent while drafting, never stated or adopted by the human | Listed here for the human; never written as intent |

An open question at handoff means the frontier was not empty; ask it before handing off rather than listing it here.
