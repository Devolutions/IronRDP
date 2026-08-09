# Pull request automation

`.github/workflows/labeler.yml` classifies ready, open pull requests and runs at most two automated reviews.
The workflow never runs an LLM for `ai-reviewed/2`; that label is terminal and requires human review.

## Review pipeline

The heavy review path uses two isolated LLM stages.
Workflow-only classifier instructions live in `.github/pr-automation/prompts/classifier.md`.
Each LLM stage similarly injects its pipeline-specific evidence and output contract from `.github/pr-automation/prompts/<stage>.md`, while reusable review methodology remains in `.agents/skills`.

1. The classifier reports whether the change is protocol-related.
   This boolean is persisted in machine-readable form on the SHA-bound `AI classification` check because review runs in a later workflow.
2. When the result is true, **Analyze protocol conformance** runs first.
   It stages the `awakecoding/openspecs` default branch into `.claude/skills/windows-protocols/` with a credentialless git fetch, without `npx`, package lifecycle scripts, or a global install.
   It also stages the repository's `.agents/skills/protocol-reviewer` skill.
   The resolved corpus commit is handed to the validation job so both stages read the same corpus even if its default branch moves.
3. A trusted job validates the handoff, including that every cited protocol ID and section heading exists in the pinned corpus.
4. **Review pull request** invokes `.agents/skills/skeptical-reviewer`.
   It never sees the corpus, receives the validated handoff as a data file, and records whether it accepted, partly accepted, or rejected the protocol findings.
5. Only the skeptical review output is published.
   Inline comments are placed only on lines the pull request adds; every other finding appears in the review body.

## Validation and recovery

Protocol-related reviews normally consume two calls against `ANTHROPIC_API_KEY_REVIEWER`, while non-protocol reviews normally consume one.
Each heavy stage validates its structured output immediately.
If an action reaches its turn limit or validation rejects the result, it resumes the same Claude session once with a six-turn output-only budget.
This retry preserves the analysis already paid for instead of starting over.
The separately isolated publication jobs still revalidate the selected result against GitHub's pull request data and the pinned protocol corpus.

If both attempts fail, the AI review count is unchanged and the pull request stays `maintainer-required`.
A neutral SHA-bound `AI automated review` check records the precise failure reason.
A classification check that predates the machine-readable protocol state is treated as unavailable, and the next classification event rewrites it.

The trusted LLM validators normalize exact empty-string representation artifacts consistently.
An invalid classifier result keeps the pull request `risk/unknown` and `maintainer-required`.
It also publishes a neutral `AI classification` check containing the local validation reason so the review remains blocked without hiding why.
A cancelled workflow does not publish a fallback state or change labels; the succeeding workflow run recomputes the classification.

## Models and cost

Each analytical stage selects its model and effort through `--model` and `--effort` in the `claude-args` step that builds `claude_args`.

| Stage | Model | Effort | Reason |
| --- | --- | --- | --- |
| Classifier | Sonnet | `low` | Runs on every push and fills a small, schema-bound triage record. |
| Protocol conformance | Opus | Default `high` | Performs the protocol analysis worth the higher cost. |
| Skeptical review | Opus | Default `high` | Evaluates correctness and the validated protocol handoff. |

The heavy stages run at most twice per pull request.
`haiku` is cheaper than Sonnet at `low` effort but supports no effort level, so the classifier does not use it.
Model names are floating aliases, so each stage tracks the latest model in its tier.

## Exclusions and limits

### Bot-authored pull requests

Bot-authored pull requests, including Dependabot's and `devolutionsbot`'s release-plz pull requests, are excluded from this workflow.
Dependabot is the sole owner of dependency and language labels, so this automation never adds, removes, or reconciles labels on bot pull requests.

### Oversized pull requests

A `size/XL` pull request has 800 or more changed source lines and is excluded from automated review before any model runs.
The classifier is also skipped so no LLM call is spent on a change that cannot be reviewed well.
Classification falls back to deterministic scope, size, first-time-contributor, and `cargo-semver-checks` results.
Every classified pull request has exactly one `size/*` label.

The workflow comments once to explain the exclusion and point to [stacked pull requests](https://docs.github.com/en/pull-requests/get-started/about-stacked-prs) for splitting dependent work.
Because stacks require every branch to live in this repository, fork authors should open separate pull requests.
The comment is removed automatically once a later push brings the change below the threshold.
Duplicate and legitimacy verdicts are model-derived, so an oversized run leaves any earlier verdict untouched rather than silently clearing it.

### Fork automation limits

Fork-origin pull requests are subject to daily UTC automation limits.
The first five pull requests from a fork author may use automation, while authors with at least 15 qualifying merged IronRDP pull requests may use ten.
Across all forks, the workflow stops LLM automation after 30 fork-origin pull requests were opened that day.
This GitHub-only global limit is best-effort under concurrent submissions.
A high-confidence non-legitimate classifier result also stops automated review and hands the pull request to a human.

## Trust boundaries

### Inputs

Every LLM stage receives the same evidence from `.github/pr-automation/fetch-pr-evidence.sh`, which runs from the trusted base checkout.
Each Claude action uses only an explicit file or skill invocation, which injects no pull request context of its own.
`Bash` is denied, so a model cannot derive a diff by itself.
Trusted workflow code writes the target head SHA and handoff-receipt status to `pr-automation-context.json` instead of interpolating them into instructions.

The evidence script writes `pr-evidence/changed-files.txt` and `pr-evidence/pull-request.diff`.
Both files are computed against the merge base so unrelated commits landing on `master` are not attributed to the pull request.
The head tree remains available in `pr-head` for surrounding context.

Before exposing that tree to filesystem-reading tools, the script removes every symlink so a pull request cannot redirect a read outside the checkout.
It also recursively removes contributor-controlled agent instruction files: `CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md`, `.claude`, `.cursor`, and `.cursorrules`.
Recursive removal is required because Claude Code discovers these files in every directory it reads, and a nested copy would escape the evidence-only boundary.
The files still appear in the diff as reviewable data rather than instructions.

### Outputs and logs

Model output is also treated as hostile.
Text published in a bot comment or review is escaped so HTML, code spans, mentions, issue references, links, images, emphasis, and related Markdown constructs render as inert prose.

Each workflow run writes a structured decision trace to its GitHub Actions logs.
The trace records event resolution, every gate and deterministic-analysis result, normalized classification and review states, and the selected label additions, removals, comments, and check mutation.
Jobs skipped by a gate appear in the final trace with their job outcome.
After every LLM stage, the log records the action outcome and its complete schema-bound structured output.
These values are untrusted pull-request-derived evidence, so they are emitted through `core.info` and never interpolated into or executed as a shell command.

## Labels

### Risk labels

The `risk/*` label states how much maintainer scrutiny a change needs, not how much an automated review is worth.
Every classified pull request has exactly one risk label:

- `risk/high` means the change substantially affects the public API surface of a core tier crate.
- `risk/medium` means a behavioral change that does not substantially alter a core public API.
- `risk/low` means a self-contained change with no cross-crate behavioral effect.
- `risk/unknown` means the classifier could not produce a valid judgment.

A `cargo-semver-checks` incompatibility always produces `risk/high`, even when the classifier is unavailable.
That check runs against the `ironrdp` facade, so every incompatibility it reports is a core public API break.
A breaking change suspected only by the classifier raises its own `low` verdict to `risk/medium` without lowering a `medium` or `high` judgment.

### Scope and kind labels

Trusted changed paths can independently add `scope/core`, `scope/web`, `scope/ffi`, and `scope/tooling`.
The classifier alone controls `scope/cross-cutting`, which requires a material behavioral, interface, or ownership boundary.
Multiple files, tests, generated companions, and manifest updates alone do not qualify.
The classifier also controls `kind/technical-debt` and documentation-only labels.
`origin/fuzzing` remains manual because paths cannot establish how a defect was discovered.

### Label setup

Run **Bootstrap pull request automation labels** once before enabling the workflow.
It creates missing labels and synchronizes the descriptions and colors declared in `.github/pr-automation/labels.json`.
It never deletes repository labels.

## Review routing

Risk measures maintainer scrutiny, so it does not decide whether a protocol change is worth reviewing.
A `protocol_related` classification always earns an automated review at any risk level.
For every other change, `risk/low` without `breaking-change` skips the review.
Duplicates, `size/XL`, a legitimacy stop, and the terminal review count stop every route.

## Review prerequisites

The first heavy review requires a successful `CI` run for the exact classified head.
A second review requires a later push and its matching successful CI run.
Manual dispatch can bypass only the CI-success requirement.
It cannot bypass draft status, duplicate or XL handling, contributor history, classification, risk, terminal state, or SHA validation.

## Secrets and versioning

The `llm-providers` environment must allow deployments from `master` and contain only `ANTHROPIC_API_KEY_CLASSIFIER` and `ANTHROPIC_API_KEY_REVIEWER`.
The workflow passes each secret only to its corresponding Claude action step.
The GitHub and Anthropic actions track their major version tags, while the Open Specifications corpus tracks its default branch so specification coverage improves without a pin bump.
