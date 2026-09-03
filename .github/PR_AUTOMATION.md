# Pull request automation

`.github/workflows/labeler.yml` classifies ready, open pull requests and calls `.github/workflows/review-pipeline.yml` for at most two automated reviews.
Automatic routes stop at `ai-reviewed/2` unless a maintainer uses force mode.
Model analysis fails closed when the reviewable pull request diff exceeds the applicable evidence limit.
The trusted `evidence-diff-attributes` policy represents reproducibly verified generated artifacts with binary-change markers.
The automation posts guidance on the pull request instead of invoking a model with partial evidence.

## Review pipeline

Classification and review use [Helmcode's OpenAI-compatible endpoint](https://api.helmcode.com/v1).
The classifier and all reviewers use `glm5.3`.

The pipeline performs these stages:

1. Prepare a SHA-bound changed-file manifest, diff, pull request context, and read-only head tree.
2. Classify risk, scope, legitimacy, duplicate likelihood, protocol relevance, and useful specialist reviewers.
3. Apply workflow-controlled routing rules and persist the canonical review plan in the `AI classification` check.
4. Run selected specialists as parallel matrix jobs, at most three at once.
5. Validate each specialist result, then aggregate the results in the canonical order `protocol`, `skeptical`, `code-compressor`.
6. Run the general reviewer as an independent reviewer and verifier.
7. Validate its candidate dispositions, findings, locations, and provenance.
8. Resolve validated state and publish through the serialized writer.

Workflow routing selects the code-compressor for every eligible review.
Protocol-related changes always require the protocol specialist.
Medium- and high-risk changes always require the skeptical specialist.
Model output cannot select parallelism.

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
The SDK retries twice on retryable responses, honors supported `Retry-After` headers, and then fails closed.

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

The evidence job fetches bounded pull request discussion and line-location data with read-only GitHub permissions.
It verifies the head before and after collection.
Final publication rechecks the current head before mutation.

## Protocol corpus

The protocol specialist reads the Microsoft Open Specifications as inert data under `review-sources/windows-protocols`.
The workflow fetches the latest `awakecoding/openspecs` master without credentials and copies only allowlisted regular Markdown files.
It excludes skills, instruction files, symlinks, submodules, executables, and lifecycle content.

Citation validation uses the same corpus commit that the specialist read, and the evidence job records its SHA in the job summary.
Every protocol ID, section number, and heading must exist in that fetched commit.
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

Automatic review runs for every non-draft pull request that passes the remaining gates.
`OWNER` and `MEMBER` authors are eligible without contributor history.
Other human authors need one qualifying merged IronRDP pull request from the same immutable author.
A qualifying pull request is any pull request from that author merged into `master`.
Automatic review requires successful CI for the exact classified head.
After the first review, a later push starts the second review when CI succeeds for that new head.
Duplicates at confidence 0.85 or greater, legitimacy triage, and `ai-reviewed/2` block automatic review.
Unavailable or invalid classification fails closed to maintainer review.

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

`size/XXL` is informational and does not block classification or review.
The evidence diff limit is 1 MiB by default.
Adding `ai-review/allow-oversized` retries classification with the model runtime's maximum 4 MiB evidence limit.
Evidence above the applicable limit fails closed without sending a partial diff to a model.

Fork-origin pull requests share a repository-wide quota of 50 pull requests per UTC day.
`OWNER` and `MEMBER` pull requests are exempt and do not count toward the quota.
Same-repository pull requests are also exempt.

## State, publication, and failure behavior

SHA-bound GitHub checks carry classification and review state between permission-isolated jobs.
SHA-bound workflow artifacts carry evidence and validated results between review-pipeline jobs.
Only the final writer mutates pull request state, and it serializes those mutations per pull request.
Model-execution jobs have read-only or empty permissions.

Two static classifier concurrency lanes allow at most two classifier jobs to invoke Helmcode at once.
The reusable `.github/workflows/review-pipeline.yml` runs under one global caller-job lock and allows at most three specialist requests at once.
The general reviewer starts only after all specialists finish, so these limits keep Helmcode usage within the five-request API-key limit.

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

The repository must contain the Actions secret named exactly `HELMCODE_GLM_API_KEY`.
The caller passes this secret explicitly to the reusable review workflow instead of relying on environment-secret forwarding.
Do not add a second provider secret or expose this key through prompts, files, outputs, logs, summaries, fixtures, diagnostics, or unrelated child processes.

Agent configuration lives in `.github/pr-automation/agents` on the base branch.
Configuration fixes model selection, prompts, schemas, methodologies, filesystem capabilities, and execution limits.
Models cannot alter these values or the Helmcode endpoint.

When Helmcode exposes GLM-5.3-Flash, consider trialing it for classification first.
If GLM-5.3 reviewer costs become too high and the trial performs well, consider migrating the reviewers too.
Keep classification in its own job so the static classifier lanes continue to bound Helmcode concurrency.

## Label setup

Run **Bootstrap pull request automation labels** once before enabling the workflow.
The bootstrap workflow creates missing labels and synchronizes descriptions and colors from `.github/pr-automation/labels.json`.
It never deletes repository labels.
