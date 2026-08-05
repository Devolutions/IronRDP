# Pull request automation

`.github/workflows/labeler.yml` classifies ready, open pull requests and runs at most two
automated reviews. The workflow never runs an LLM for `ai-reviewed/2`; that label is terminal and
requires human review.

A heavy review is split into two isolated LLM stages. The classifier reports whether the change is
protocol-related, and that boolean is persisted in machine-readable form on the SHA-bound
`AI classification` check because the review route runs in a later workflow run. When it is true,
**Analyze protocol conformance** runs first: it stages the `awakecoding/openspecs` default branch
into `.claude/skills/windows-protocols/` with a credentialless git fetch — no `npx`, package
lifecycle script, or global install — and returns a structured protocol handoff. The commit it
resolved is handed to the validation job, so both stages read the same corpus even if the corpus
moves in between. A trusted job then validates that handoff, including that every cited protocol ID
and section heading exists in that corpus. **Review pull request** is the skeptical stage: it never
sees the corpus, receives the validated handoff as a data file, and must record whether it accepted,
partly accepted, or rejected the protocol findings. Only its output is published, and inline
comments are placed only on lines this pull request actually adds; any other finding is published in
the review body instead.

Protocol-related reviews therefore consume two calls against `ANTHROPIC_API_KEY_REVIEWER`;
non-protocol reviews consume one. If the protocol stage fails, is cancelled, is malformed, or cites
a specification section that does not exist, the skeptical stage does not run, the AI review count
is unchanged, and the PR stays `maintainer-required`. A classification check that predates the
machine-readable protocol state is treated as unavailable; the next classification event rewrites
it.

Each stage selects its model and effort through `--model` and `--effort` in the `claude-args` step
that builds its `claude_args`. The classifier runs Sonnet at `low` effort: it runs on every push and
only has to fill a small, schema-bound triage record, so it buys Sonnet-class judgement at a
fraction of the default token spend. Both heavy stages run Opus at its default `high` effort, since
protocol conformance and skeptical judgement are the work worth paying for and they run at most
twice per pull request. `haiku` is cheaper than Sonnet at `low` effort but supports no effort level
at all, which is why the classifier does not use it. The model names are floating aliases, so each
stage tracks the latest model of its tier.

Bot-authored pull requests, including Dependabot's and `devolutionsbot`'s release-plz PRs, are
excluded from this workflow. Dependabot is the sole owner of dependency and language labels, so
this automation never adds, removes, or reconciles labels on bot pull requests.

Every LLM stage is given the same evidence, prepared by `.github/pr-automation/fetch-pr-evidence.sh`
running from the trusted base checkout. The action is used in explicit-prompt mode, which injects no
pull request context of its own, and `Bash` is denied, so a model cannot derive a diff by itself.
The script therefore writes `pr-evidence/changed-files.txt` and `pr-evidence/pull-request.diff`,
both computed against the merge base so that unrelated commits landing on `master` are not
attributed to the pull request, and the head tree stays available in `pr-head` for surrounding
context. Before exposing that tree to filesystem-reading tools, it removes all symlinks so a pull
request cannot redirect a read outside the checkout. It also removes contributor-controlled agent
instruction files — `CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md`, `.claude`, `.cursor`,
`.cursorrules` — recursively rather than only at the checkout root, because Claude Code discovers
them in every directory it reads and a nested copy would otherwise escape the evidence-only
boundary. Those files still appear in the diff, where they are reviewable data rather than
instructions.

Model output is likewise treated as hostile on the way out. Text published in a bot comment or
review is escaped so that HTML, code spans, mentions, issue references, and the Markdown constructs
that produce links, images, and emphasis all render as inert prose.

Each workflow run writes a structured decision trace to its GitHub Actions logs. It records event
resolution, all gate and deterministic-analysis results, normalized classification and review states,
and the selected label additions, removals, comments, and check mutation. Jobs skipped by a gate are
included in the final trace with their job outcome. After every LLM
stage, the log also records the action outcome and its complete schema-bound structured output. These
values are untrusted pull-request-derived evidence: they are emitted through `core.info`, never
interpolated into a shell command or executed.

The `risk/*` label states how much maintainer scrutiny a change needs, not how much an automated
review is worth. Every classified pull request has exactly one risk label. `risk/high` means the
change substantially affects the public API surface of a core tier crate; `risk/medium` means a
behavioural change that does not substantially alter a core public API; `risk/low` means a
self-contained change with no cross-crate behavioural effect; and `risk/unknown` means the
classifier could not produce a valid judgement. A `cargo-semver-checks` incompatibility always
produces `risk/high`, even when the classifier is unavailable, because that check runs against the
`ironrdp` facade and every incompatibility it reports is a core public API break. A breaking change
that only the classifier suspects raises its own `low` verdict to `risk/medium` rather than lowering
its `medium` or `high` judgement.

Because risk measures maintainer scrutiny, it does not decide whether a protocol change is worth
reviewing: a `protocol_related` classification always earns an automated review, at any risk level.
For everything else, `risk/low` without `breaking-change` skips the review. Duplicates, `size/XL`,
a legitimacy stop, and the terminal review count still stop every route.

A `size/XL` pull request, meaning 800 or more changed source lines, is excluded from automated
review deterministically, before any model runs. The classifier is skipped along with the
reviewers, so no LLM call is spent on a change that cannot be reviewed well anyway; classification
falls back to deterministic scope, size, first-time-contributor, and `cargo-semver-checks` results.
Every classified pull request has exactly one `size/*` label. The workflow comments once to explain the
exclusion and to point at [stacked pull requests](https://docs.github.com/en/pull-requests/get-started/about-stacked-prs)
for splitting dependent work, noting that stacks require every branch to live in this repository so
fork authors should open separate pull requests. The comment is removed automatically once a later
push brings the change below the threshold. Duplicate and legitimacy verdicts are model-derived, so
an oversized run leaves any earlier verdict untouched rather than silently clearing it.

Run **Bootstrap pull request automation labels** once before enabling the workflow. It creates any
missing labels and synchronizes the descriptions and colors declared in
`.github/pr-automation/labels.json`; it never deletes repository labels.

Trusted changed paths can independently add `scope/core`, `scope/web`, `scope/ffi`, and
`scope/tooling`. The classifier alone controls `scope/cross-cutting`, which requires a material
behavioral, interface, or ownership boundary; multiple files, tests, generated companions, and
manifest updates alone do not qualify. The classifier also controls `kind/technical-debt` and
documentation-only labels. `origin/fuzzing` remains manual because paths cannot establish how a
defect was discovered.

The first heavy review requires a successful `CI` run for the exact classified head. A second
review requires a later push and its matching successful CI run. Manual dispatch can bypass only
the CI-success requirement; it cannot bypass draft status, duplicate/XL handling, contributor
history, classification, risk, terminal state, or SHA validation.

Fork-origin PRs are subject to daily UTC automation limits. The first five PRs from a fork author
may use automation; authors with at least 15 qualifying merged IronRDP PRs may use ten. Across all
forks, the workflow stops LLM automation after 30 fork-origin PRs were opened that day. This
GitHub-only global limit is best-effort under concurrent submissions. A high-confidence
non-legitimate classifier result also stops automated review and hands the PR to a human.

`llm-providers` must allow deployments from `master` and contain only
`ANTHROPIC_API_KEY_CLASSIFIER` and `ANTHROPIC_API_KEY_REVIEWER`. The workflow passes each secret
only to its corresponding Claude action step. The GitHub and Anthropic actions track their major
version tags, and the Open Specifications corpus tracks its default branch, so specification
coverage improves without a pin bump.
