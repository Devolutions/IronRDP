# Pull request automation

`.github/workflows/labeler.yml` classifies ready, open pull requests and calls `.github/workflows/review-pipeline.yml` for at most two automated reviews.
Automatic routes stop at `ai-reviewed/2` unless a maintainer uses force mode.
Manual `workflow_dispatch` requests and forced reviews require a successful GitHub Actions-owned `AI classification` check for the current head with valid machine state.
They fail visibly before any reviewer starts when that prerequisite is missing, stale, or invalid; automatic CI and classification-complete races instead skip normally.
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
8. Report every stage outcome, failure reason, and metric back to the caller.
9. Resolve validated state and publish through the serialized writer.

`.github/workflows/review-pipeline.yml` is reusable and `workflow_call` is its only trigger.
The caller owns the global concurrency lock, the reviewer selection, and publication.
The pipeline owns evidence, specialists, aggregation, the general review, validation, and stage recovery.

`required-reviewers` names the specialists that must succeed, and the caller is authoritative.
The pipeline only falls back to the classification gate when that input is absent.
The evidence job settles that question once per invocation, and every later stage reads the resolved list rather than interpreting the policy again.
A stage that cannot read the resolved list treats every selected reviewer as mandatory.

Workflow routing selects the code-compressor for every eligible review.
Protocol-related changes always require the protocol specialist.
Medium- and high-risk changes always require the skeptical specialist.
Model output cannot select parallelism.
Three simultaneous reviewer requests is a provider allocation, not a reviewer cap: a larger selection runs in further batches.

Specialists use one bounded candidate schema.
Each candidate binds to the expected head SHA, a configured reviewer ID, a changed path, an optional added-line range, a severity, a question flag, and a unique finding ID.
Protocol candidates also carry structured protocol references.
One specialist never receives another specialist's output.

The general reviewer independently inspects the pull request, attempts to falsify every candidate, and records exactly one `accepted`, `refined`, or `rejected` disposition per candidate.
It can merge overlapping candidates and add findings that no specialist reported.
Only the validated general-review result can be published.

## Reviewer output validation

Every provider request a reviewer invocation makes is retried up to four times after its initial attempt.

`.github/pr-automation/agent-validator.js` is the trusted review validator the model runtime calls.
The runtime validates JSON and the output schema, then hands the parsed candidate to this module together with bounded metadata naming the stage, the reviewer, the expected SHAs, and the trusted context files.
The validator reads the changed-file manifest, the protocol corpus, and the specialist aggregate itself, from paths only the trusted workflow can write.

The validator distinguishes two outcomes.
A wrong head SHA, an unchanged path, a malformed line range, an unverifiable protocol citation, or a missing candidate disposition is correctable, so the runtime repairs the output inside the same conversation, at most twice.
A stale or unavailable trusted input is not correctable, so the stage fails immediately instead of burning repair attempts.
Repair may correct a finding but may never drop one, and a stage fails when it cannot produce valid output.

Repair happens inside one invocation, so the pipeline never restarts a reviewer to fix its output and keeps no checkpoints of its own.

## Stage recovery

A transient provider failure costs one extra invocation of that stage, not a replay of the review.

The runtime decides whether a failure is retryable, and the pipeline keeps no failure taxonomy of its own.
A retryable stage waits for `retry-delay-seconds` (120 by default), re-decides review eligibility against the pull request as it is after the delay, and runs exactly once more inside the same job.
That recheck repeats the caller's own gate: open state, exact head and base, draft state, review policy labels, the classification bound to this head and its reviewer set, an already-published review, the newest CI run, contributor eligibility, the fork quota, and the evidence size limit still in force.
A caller `force` bypasses review policy and CI, and never the safety checks.
A declined retry is reported with its reason.
Every stage that already succeeded keeps its result, so a recovered review repeats only the work that failed.
Recovery is bounded to one delayed retry per stage, so a stage reports at most two attempts and the pipeline cannot loop.

The runtime marks transient provider failures retryable: timeouts, dropped connections, conflicts, rate limits, and service errors.
Exhausted output repair is settled: the runtime already corrected inside the same conversation, so repeating the request cannot help.
An unreachable API means the retry is not attempted, because a review that cannot be proved wanted is not worth a second request.

Evidence is prepared once and every stage, including a delayed retry, reads those exact bytes.
Artifacts stay inside the pipeline execution, and a later caller run starts fresh rather than inheriting results across runs.

The pipeline returns every failed stage with its reason and failure category, not just the first failure.
It also returns per-stage token usage, elapsed time, request-retry count, output-repair count, the number of recovered stages, and, for a recovered stage, the failure its first attempt reported.
Unmeasured metrics are reported as missing rather than as zero, and a retried stage is charged for both of its attempts.
The aggregate marks itself incomplete whenever a stage that called a provider could not account for its usage.
Each stage records whether it called a provider, so the caller's totals are the pipeline's own.

`.github/pr-automation/review-report.js` defines the one report schema the pipeline writes and the caller reads, so the two sides cannot drift.
The caller parses it with `parseReport`, which never throws and never reads a malformed or unsupported report as success.

## Visible finding format and sources

Published findings use severity as the only ranking signal:

| Severity | Indicator |
| --- | --- |
| `critical` | `:purple_circle:` |
| `high` | `:red_circle:` |
| `medium` | `:orange_circle:` |
| `low` | `:yellow_circle:` |

Questions append `:question:` after the severity indicator.
A successful review with no findings shows `:green_circle:` in its main comment.

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
Its workflow-controlled configuration enforces turn, tool-call, path, byte, line, recursion, result, request-timeout, request-retry, output-size, and output-repair limits.
The caller can opt into a trusted validator from the workflow checkout and pass bounded invocation metadata.
The action validates JSON and schema before the validator, then preserves the conversation for bounded correction turns.
For validator checks, `previousCandidate` is the earliest JSON-parsed candidate in the repair sequence, including a value that did not pass local schema and may be any JSON type.
Validator-directed corrections may use only necessary bounded read-only evidence lookup, while invalid output remains terminal after its configured repair budget.
The SDK adapter owns retries of the same request within its single configured retry budget and honors valid `Retry-After` delays.
Known transient statuses retry and known terminal statuses stop despite provider retry hints; unrecognized responses use SDK policy.
The configured request timeout bounds individual network attempts and non-success response bodies.
A known response-body transport failure that escapes SDK retries is categorized as a stage-recoverable connection failure.
Strict provider JSON Schema mode is opt-in only for a configured supported endpoint; local validation always remains enforced.
It reports safe activity, per-attempt duration, retry and repair counts, finish reason, available token usage with completeness state, and machine-readable terminal or transient failure categories through one diagnostics output.

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
A recovery attempt restores the pinned evidence instead of refetching it, and still reverifies that the pull request is open at the same head.
Final publication rechecks the current head before mutation.

## Protocol corpus

The protocol specialist reads the Microsoft Open Specifications as inert data under `review-sources/windows-protocols`.
The workflow fetches the latest `awakecoding/openspecs` master without credentials and copies only allowlisted regular Markdown files.
It excludes skills, instruction files, symlinks, submodules, executables, and lifecycle content.

Citation validation uses the same corpus commit that the specialist read, and the evidence job records its SHA in the job summary.
Every protocol ID, section number, and heading must exist in that fetched commit.
A recovery attempt restores the pinned corpus instead of refetching the latest master, so recovering a review cannot invalidate the work it is recovering.
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
Force mode can override policy gates for an open pull request at its current head after a trusted, valid classification for that exact head selects its reviewers.
Force mode never bypasses classification validity, evidence retrieval, output validation, filesystem restrictions, protocol citation validation, or stale-head checks.

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
Attempt-scoped workflow artifacts carry evidence and validated results between review-pipeline jobs and across recovery attempts.
Only the final writer mutates pull request state, and it serializes those mutations per pull request.
Model-execution jobs have read-only or empty permissions.

Four static classifier lanes allow at most four classifier jobs to invoke Helmcode at once.
Seven static caller-job lanes lock each reusable review pipeline from evidence through its result.
Each pipeline allows at most three specialist requests at once, for at most 25 model requests across the classifier and reviewer lanes.
The general reviewer starts only after all specialists finish.

Inline comments target only validated added lines.
Other findings appear in the review body.
All model prose is escaped to neutralize Markdown, HTML, mentions, issue references, and links.

Specialist failures are recorded explicitly.
An optional specialist failure completes the review with reduced coverage and names the unavailable reviewer in the review and check.
Detailed failure reasons appear only in the workflow summary.
Every failed stage is reported, not only the first one.
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

When Helmcode exposes GLM-5.3-Flash, consider trialing it for classification first.
If GLM-5.3 reviewer costs become too high and the trial performs well, consider migrating the reviewers too.
Keep classification in its own job so the static classifier lanes continue to bound Helmcode concurrency.

## Label setup

Run **Bootstrap pull request automation labels** once before enabling the workflow.
The bootstrap workflow creates missing labels and synchronizes descriptions and colors from `.github/pr-automation/labels.json`.
It never deletes repository labels.
