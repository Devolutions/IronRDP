# Before and after: Devolutions Gateway `AGENTS.md`

`git diff 6b906d7d b6c98d77 -- AGENTS.md` in Devolutions/devolutions-gateway, PR #2091.
151 lines became 27.
IronRDP's equivalent is Devolutions/IronRDP#2131 (155 to 38).
The result is `git show b6c98d77:AGENTS.md`: five headings (intent files, writing for the repository, Rust, untested-property comments, Git) under a one-line skills opener.

## What was cut, and why it was cuttable

| Block in the old file | Verdict | Evidence |
|---|---|---|
| `Issue: #456 (GitHub issue)` as a valid footer | Wrong | `cliff.toml` renders every `Issue:` value as a Jira URL; four forms of the mistake in the last 300 commits |
| Scopes `dgw`, `agent`, `jetsocat`, `webapp`; "library scopes `*deps*`, `*openapi*`, `*npm*`" | Stale | `installer` and `agent-installer` missing; `cliff.toml` suppresses only exact `chore(deps)`/`build(deps)`; the `commit-scope` skill already had the right table |
| Branch naming (`DGW-338-...`, `fix/123-description`) | Never a convention | 2 of the last 60 merged PR branches carried a Jira key |
| ~70 lines of PR title/body/footer/examples | Duplicated | `commit-and-pr-conventions` and `commit-scope` skills |
| ~55 lines of Rust style (errors, logging, comments, invariants, size constants) | Duplicated, lossy | a summary of IronRDP `STYLE.md`, vendored by the same PR and served by the `rust-style` skill |
| `## Project Overview` workspace listing | Discoverable | `Cargo.toml`, `webapp/pnpm-workspace.yaml` |
| `## Additional Resources` (`Main README: README.md`, `Cookbook: docs/COOKBOOK.md`) | Discoverable | one `ls` |
| TypeScript and PowerShell command lists | Discoverable | `webapp/package.json` scripts, `powershell/*.ps1` |
| `Tests go in testsuite/tests/, helpers in testsuite/src/`; `Consider proptest, rstest` | Discoverable | the tree; replaced by one pointer to `testsuite/AGENTS.md` |
| `Follow standard .NET conventions.` / `Follow standard PowerShell conventions.` | Content-free | |
| `cargo clippy --workspace --tests -- -D warnings`, `cargo test --workspace` | Plain command | CI annotations print the command to run |

## What was kept or added, and the test it passed

| Line | Why it earns its place |
|---|---|
| Nightly rustfmt | Kept at the time as a one-round gotcha; under the feedback-round test it is borderline, since CI reports the fix in one round, and the maintainer is inclined to drop it. IronRDP dropped it because it formats with stable: verify before copying |
| `#[expect(..., reason = "...")]` with its reason | Policy; the old text said `THINK before suppressing` without the why |
| Pointer to `testsuite/AGENTS.md` | Non-standard Cargo layout; details live in the nested file (26 lines: run the binary, dependencies are a design signal, fewer stronger tests, timing tests run serially) |
| `Issue: DGW-123` | The shared commit skill defers footer format to the repository; the one-line form stays here and the full rules live in the repo-local `commit-footer` skill the line points to |
| Intent files, writing for the repository, untested-property comments | Policy no agent infers and no skill holds; taken from IronVNC and picky-rs |
| Skills opener | For agents that do not auto-discover `.agents/skills/` |

## Maintainer pushbacks, verbatim

From Gateway, IronVNC, and sspi-rs `AGENTS.md` reviews.

- "it contains redundant information that is better derived by each agents on demand"
- "Remove everything that can be explored naturally (e.g.: layout section)"
- "I would not include any 'obvious' cargo command, it may be worth keeping the line about using nightly for formatting, and to think before silenting lints."
- "I don't think we even need the introduction about what is Devolutions Gateway"
- "I think we can refer to rust-style in the top-level AGENTS.md, but I would keep rust-style-reviewer mentioned only in the code-review skill."
- "Don't we risk to have the agent pull off the full file? Is it better to refer to the STYLE.md file only in the reviewer skill?"
- "Also verify the Git section does not duplicate the other matching skills."
- "The best place for that may be in the matching skill instead of AGENTS.md"
- "The property is that we run them serially, it may be good to explain that" (a timing-tests section had described fixed delays instead of the property)
- "I think this is not necessary, agents already do it without instructions" ([sspi-rs #763](https://github.com/Devolutions/sspi-rs/pull/763#discussion_r4195986262))
- "Since this can be derived easily from the workspace Cargo.toml, I would tend to remove that" ([sspi-rs #763](https://github.com/Devolutions/sspi-rs/pull/763#discussion_r4196024203))
- "Do you think of a specific anti-pattern?" and "I need some more meat on this to understand what that is concretely. What are we asking the agent to do?" ([sspi-rs #763](https://github.com/Devolutions/sspi-rs/pull/763#discussion_r4196018926))
- "I think this is better suited as a review-style skill, rather than the top-level AGENTS.md" ([sspi-rs #763](https://github.com/Devolutions/sspi-rs/pull/763#discussion_r4195993368))
- "It's likely enough to suggest `cargo xtask check wasm`, and remove the note about native threads, filesystem, etc. Even if the agent would use such APIs, it would be blocked by `cargo xtask check wasm`." ([IronVNC #1544](https://github.com/Devolutions/IronVNC/pull/1544#discussion_r4024093374))
- "I would remove the heading of all files where it's not really necessary. This is read by agents, and it's found in a folder called 'ironvnc-fuzzing'." ([IronVNC #1542](https://github.com/Devolutions/IronVNC/pull/1542#discussion_r4013920553))
- The positive form: "Perfect, there is no way the agent can learn about this if we don't write it in the AGENTS.md" ([sspi-rs #763](https://github.com/Devolutions/sspi-rs/pull/763#discussion_r4196027485))

## Resulting reference graph

```text
AGENTS.md ──► rust-style skill (no pointer onward)
code-review skill ──► rust-style-reviewer skill ──► STYLE.md (only when a rule's intent is unclear)
AGENTS.md ──► testsuite/AGENTS.md
AGENTS.md ──► commit-footer skill
```

Each file is loaded at the latest point where it pays for itself.
