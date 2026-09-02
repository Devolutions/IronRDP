# Pull request automation

`.github/workflows/labeler.yml` classifies ready, open pull requests and runs at most two automated reviews.
Automatic routes stop at `ai-reviewed/2` unless a maintainer uses force mode.
Model analysis also fails closed when the complete pull request diff exceeds the 1 MiB evidence limit.
The automation posts guidance on the pull request instead of invoking a model with partial evidence.

## Review pipeline

Classification and review use [Helmcode's OpenAI-compatible endpoint](https://api.helmcode.com/v1).
The classifier and all reviewers temporarily use `deepseek-v4-flash` until a GLM key is available.

The pipeline performs these stages:

1. Prepare a SHA-bound changed-file manifest, diff, pull request context, and read-only head tree.
2. Classify risk, scope, legitimacy, duplicate likelihood, protocol relevance, and useful specialist reviewers.
3. Apply workflow-controlled routing rules and persist the canonical review plan in the `AI classification` check.
4. Run selected specialists sequentially in the order `protocol`, `skeptical`, then `code-compressor`.
5. Validate each specialist result and write `validated-specialist-findings.json`.
6. Run the general reviewer as an independent reviewer and verifier.
7. Validate its candidate dispositions, findings, locations, and provenance.
8. Resolve validated state and publish through the serialized writer.

Workflow routing selects the code-compressor for every eligible review.
Protocol-related changes always require the protocol specialist.
Medium- and high-risk changes always require the skeptical specialist.
IronRDP configures sequential execution only, and model output cannot select parallelism.

Specialists use one bounded candidate schema.
Each candidate binds to the expected head SHA, a configured reviewer ID, a changed path, an optional added-line range, and a unique finding ID.
Protocol candidates also carry structured protocol references.
One specialist never receives another specialist's output.

The general reviewer independently inspects the pull request, attempts to falsify every candidate, and records exactly one `accepted`, `refined`, or `rejected` disposition per candidate.
It can merge overlapping candidates and add findings that no specialist reported.
Only the validated general-review result can be published.

## Visible finding sources

Workflow code derives a visible prefix from validated source references.
The model cannot provide or override the prefix.

Examples include:

```text
[protocol]
[skeptical]
[code-compressor]
[protocol + skeptical]
[general]
```

Specialist-derived findings list every distinct source category in deterministic order.
Findings discovered only by the general reviewer use `[general]`.
The prefix appears in both inline comments and review-body findings.
Model-generated titles and rationales remain untrusted and are escaped independently.

## Model runtime

`.github/actions/openai-agent` is a bundled JavaScript action built on the official OpenAI SDK.
It loads a workflow-controlled agent configuration, prompt, output schema, methodology, and filesystem capability list.
It exposes only `read_file`, `list_files`, and `search_text`.
It enforces turn, tool-call, path, byte, line, recursion, result, timeout, and retry limits.
It validates JSON against the configured schema and permits one tools-disabled correction completion.

The action exposes no command execution, writes, Git operations, GitHub APIs, environment access, arbitrary network access, or generic URL fetching.
It logs bounded metadata only and never logs prompts, pull request content, tool arguments, tool results, model responses, provider response bodies, or credentials.

The action directory contains no IronRDP prompts, reviewer identities, routing, OpenSpecs handling, state resolution, or publication policy.
It can move to the public Devolutions Actions repository without deleting repository-specific code.
Extraction should preserve the bundled artifact, lockfile, tests, input contract, and consumer-controlled configuration.

## Evidence and filesystem boundaries

`.github/pr-automation/fetch-pr-evidence.sh` runs from the base checkout without repository credentials.
It binds evidence to the resolved base and head SHAs and computes the merge-base diff.
The untrusted head tree is available only for surrounding context.

Before model access, the script removes all symlinks and recursively removes denylisted contributor-controlled agent instructions and provider metadata.
Those files remain visible in the authoritative diff as reviewable changes.
The runtime rejects absolute paths, traversal, `.git`, symlinks, junctions, realpath escapes, binary files, oversized files, and paths outside explicit capabilities.

The review job fetches bounded pull request discussion and line-location data with read-only GitHub permissions.
It verifies the head before and after collection.
Final publication rechecks the current head before mutation.

## Protocol corpus

The protocol specialist reads the Microsoft Open Specifications as inert data under `review-sources/windows-protocols`.
The workflow pins a reviewed `awakecoding/openspecs` commit, fetches that exact SHA without credentials, and copies only allowlisted regular Markdown files.
It excludes skills, instruction files, symlinks, submodules, executables, and lifecycle content.

Citation validation uses the same corpus commit that the specialist read.
Every protocol ID, section number, and heading must exist at that exact commit.
An unavailable corpus, protocol specialist, or protocol validation blocks publication for a mandatory protocol review.

## Classification and review policy

Risk labels express required maintainer scrutiny:

| Label | Meaning |
| --- | --- |
| `risk/high` | Substantial core public API impact. |
| `risk/medium` | Behavioral change without substantial core public API impact. |
| `risk/low` | Self-contained change without cross-crate behavioral impact. |
| `risk/unknown` | No valid classification was available. |

`cargo-semver-checks` incompatibility forces `risk/high`.
A model-suspected breaking change promotes `risk/low` to `risk/medium`.
Path rules can add `scope/core`, `scope/web`, `scope/ffi`, and `scope/tooling`.
The classifier controls `scope/cross-cutting`, `kind/technical-debt`, and documentation-only classification.

Automatic review requires successful CI for the exact classified head and at least three qualifying merged IronRDP pull requests from the author.
A second review requires a later push and matching successful CI.
Duplicates, legitimacy triage, the terminal review count, and oversized changes without explicit opt-in block automatic review.
Non-protocol `risk/low` changes without a breaking-change signal skip review.

Bot-authored pull requests do not run automatic routes or label reconciliation.
Force mode can override policy gates for an open pull request at its current head.
Force mode never bypasses evidence retrieval, output validation, filesystem restrictions, protocol citation validation, or stale-head checks.

## Size and fork limits

Size uses the larger bucket from counted changed lines or touched files:

| Label | Counted changed lines | Touched files |
| --- | ---: | ---: |
| `size/XS` | 0-49 | 1-2 |
| `size/S` | 50-199 | 3-5 |
| `size/M` | 200-449 | 6-10 |
| `size/L` | 450-899 | 11-20 |
| `size/XL` | 900-1299 | 21-49 |
| `size/XXL` | 1300 or more | 50 or more |

`size/XXL` skips model execution unless a maintainer adds `ai-review/allow-oversized`.
The opt-in waives only the size gate.
CI, quota, duplicate, legitimacy, contributor-history, and review-count gates still apply.

Fork-origin pull requests have daily UTC quotas.
The default author quota is five pull requests, established contributors can use ten, and the best-effort repository-wide quota is 30.

## State, publication, and failure behavior

SHA-bound GitHub checks carry classification and review state between permission-isolated jobs.
Only the final writer mutates pull request state, and it serializes those mutations per pull request.
Model-execution jobs have read-only or empty permissions.

Inline comments target only validated added lines.
Other findings appear in the review body.
All model prose is escaped to neutralize Markdown, HTML, mentions, issue references, and links.

Specialist failures are recorded explicitly.
A mandatory specialist failure, invalid aggregate, invalid final review, exhausted limit, provider failure, or unavailable evidence fails closed to `maintainer-required`.
Stale heads stop publication without mutation.
Failed reviews do not increment the automated review count.
Cancelled runs do not publish fallback state.

## Configuration and upgrades

Every Helmcode job declares:

```yaml
environment: llm-providers
```

Every model invocation receives its credential through:

```yaml
api-key: ${{ secrets.HELMCODE_GLM_API_KEY }}
```

The environment must contain the secret named exactly `HELMCODE_GLM_API_KEY`.
Do not add a second provider secret or expose this key through prompts, files, outputs, logs, summaries, fixtures, diagnostics, or unrelated child processes.

Agent configuration lives in `.github/pr-automation/agents` on the base branch.
Configuration fixes model selection, prompts, schemas, methodologies, filesystem capabilities, and execution limits.
Models cannot alter these values or the Helmcode endpoint.

To restore GLM, update the base-branch `model` fields after the GLM key is available and the action's mocked provider and schema tests pass unchanged.
Keep classification in a separate job while concurrency isolation remains useful.

## Label setup

Run **Bootstrap pull request automation labels** once before enabling the workflow.
The bootstrap workflow creates missing labels and synchronizes descriptions and colors from `.github/pr-automation/labels.json`.
It never deletes repository labels.
