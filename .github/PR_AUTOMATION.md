# Pull request automation

`.github/workflows/pr-automation.yml` classifies ready, open pull requests and calls `.github/workflows/review-pipeline.yml` for at most two automated reviews.
Automatic review stops at `ai-reviewed/2` unless a maintainer uses force mode; classification keeps running.
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
2. Classify risk, scope, legitimacy, overlap with another pull request, protocol relevance, and useful specialist reviewers.
3. Apply workflow-controlled routing rules and persist the canonical review plan in the `AI classification` check.
4. Run selected specialists as parallel matrix jobs, at most three at once.
5. Validate each specialist result, then aggregate the results in the canonical order `protocol`, `skeptical`, `code-compressor`.
6. Run the general reviewer as an independent reviewer and verifier.
7. Validate its candidate dispositions, findings, locations, and provenance.
8. Report every stage outcome, failure reason, and metric back to the caller.
9. Resolve validated state and publish through the serialized writer.

`.github/workflows/review-pipeline.yml` is reusable and `workflow_call` is its only trigger.
The caller owns the concurrency lane, reviewer selection, and publication.
The pipeline owns evidence, specialists, aggregation, the general review, and validation.

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
Schema line and pull-request numbers stop at the signed 32-bit GitHub API integer maximum, while validators retain safe-integer checks.
One specialist never receives another specialist's output.
The validated aggregate of all selected specialists is bounded to 1 MiB, which accommodates every maximum schema-valid specialist result.
Review payloads move between action, validation, reusable workflow, caller, and writer as workspace files and one-day artifacts rather than environment variables or job outputs.
The classifier alone uses a direct output because a conservatively escaped maximum schema payload stays below 32 KiB, well below Linux's 128 KiB per-entry limit.

The general reviewer independently inspects the pull request, attempts to falsify every candidate, and records exactly one `accepted`, `refined`, or `rejected` disposition per candidate.
A candidate is one entry in the findings of a reviewer the aggregate reports as valid, so a reviewer that failed or reported nothing contributes none.
It can merge overlapping candidates and add findings that no specialist reported.
Only the validated general-review result can be published.
Final review validation renders the escaped GitHub review payload and rejects output above GitHub's 65,536-character review-body limit before publication.

## Reviewer output validation

Every logical model call can make up to four retry HTTP attempts after its initial HTTP attempt.

`.github/pr-automation/output-normalizer.js` is the trusted stage-specific canonicalizer.
It projects unknown properties only at strict schema-object boundaries and normalizes only product-declared non-authoritative summary tails before local schema validation.
The classifier uses its normalizer without a semantic validator.
Specialist and general stages normalize first, validate the local JSON Schema, then call `.github/pr-automation/agent-validator.js` with bounded metadata naming the stage, reviewer, expected SHA, and trusted context files.
The validator reads the changed-file manifest, the protocol corpus, and the specialist aggregate itself, from paths only the trusted workflow can write.

The validator distinguishes two outcomes.
A wrong head SHA, an unchanged path, a malformed line range, an unverifiable protocol citation, or a missing candidate disposition is correctable, so the runtime repairs the output inside the same conversation, at most twice.
Final-review rejections describe validation failures without quoting model text.
Disposition-map errors are reported together, so repairs do not have to discover missing candidates one at a time.
A stale or unavailable trusted input is not correctable, so the stage fails immediately instead of burning repair attempts.
Repair may correct a finding but may never drop one, and a stage fails when it cannot produce valid output.

Repair happens inside one invocation, so the pipeline never restarts a reviewer to fix its output and keeps no checkpoints of its own.

## Stage execution

Transient failures retry only within the logical model call that experienced them.
Reviewer stages never restart, so each pipeline stage has one outcome and one set of diagnostics.
Evidence is prepared once and every stage reads the same bytes.
Artifacts stay inside the pipeline execution, and a later caller run starts fresh rather than inheriting results across runs.

The pipeline returns every failed stage with its reason and failure category, not just the first failure.
It also returns per-stage token usage, elapsed time, request-retry count, and output-repair count.
Unmeasured metrics are reported as missing rather than as zero.
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
Its workflow-controlled configuration enforces turn, tool-call, path, byte, line, recursion, result, stream-idle, stage, request-retry, and output-repair limits.
The runtime bounds raw streamed bodies and accepted serialization independently of schema value bounds.
For large review schemas, accepted output is written once to a workflow-controlled target in the action's dedicated workspace directory; the action rejects symlinks, existing targets, and paths outside that directory.
The target is action input rather than an echoed success output, and workflow consumers select their known path from the action step outcome.
The caller can opt into a trusted synchronous normalizer and a trusted validator from the workflow checkout and pass bounded validator metadata.
The action normalizes after parsed-value Unicode sanitization, then validates a genuine JSON value, serialization bound, and schema before the validator.
For validator checks, `previousCandidate` is the earliest canonical JSON-parsed candidate in the repair sequence, including a value that did not pass local schema and may be any JSON type.
Validator-directed corrections may use only necessary bounded read-only evidence lookup, while invalid output remains terminal after its configured repair budget.
One model turn is one logical model call.
The action owns retry HTTP attempts for that logical call through the SDK's public completion API and honors valid `Retry-After` delays.
Pre-header failures, status failures, and interrupted streams share one retry budget and discard partial response state before resending the unchanged history.
Known transient statuses retry, while known terminal statuses stop despite provider retry hints.
Every request streams under a per-attempt raw-body and idle-progress budget plus one monotonic stage deadline that also covers backoff, tools, validation, and repair.
The checked-in profiles request high reasoning effort and leave the provider token cap unset.
The runtime assembles reasoning, content, usage, finish reason, and indexed tool-call fragments, and no tool executes until the complete batch passes preflight.
Strict provider JSON Schema mode is opt-in only for a configured supported endpoint; local validation always remains enforced.
It reports safe activity, deterministic logical-call and HTTP-attempt indices, request and tool-result byte counts, retry and repair counts, finish reason, available token usage with completeness state, bounded provider error codes, and machine-readable terminal or transient failure categories through one diagnostics output.
The workflow job timeout covers one reviewer stage plus cleanup so a bounded failure can still be persisted.

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
Every non-bot author is eligible immediately, including first-time contributors; there is no prior-merge requirement.
Automatic review requires successful CI for the exact classified head.
After the first review, a later push starts the second review when CI succeeds for that new head.
Legitimacy triage and `ai-reviewed/2` block automatic review.
A suspected overlap with another pull request is advisory: at confidence 0.85 or greater it adds `triage/overlap` and a non-blocking comment, and review proceeds under the usual gates.
The classifier reports possible shared scope in `overlap`, using candidate titles and truncated bodies.
Unavailable or invalid classification fails closed to maintainer review.

`maintainer-required` marks a pull request whose next step belongs to a maintainer.
A review applies it when it reports no findings and withdraws it when it reports findings.
Once `ai-reviewed/2` is set, classification applies it on the next push, because automatic review has stopped.

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
Workflow artifacts carry evidence and validated results between review-pipeline jobs.
Only the final writer mutates pull request state, and it serializes those mutations per pull request.
Model-execution jobs have read-only or empty permissions.
The run summary links the pull request the run resolved.

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
