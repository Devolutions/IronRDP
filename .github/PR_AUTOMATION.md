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
is unchanged, and the PR stays `human-required`. A classification check that predates the
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

Bot-authored pull requests, such as dependabot's, stop at deterministic labelling. They receive
path and size labels plus `human-required`, and no model ever runs: no classifier, no semver or
SSPI heuristic, no risk label, and no review route. Bots are machine-generated and arrive in bulk,
so model capacity spent on them buys no reviewable judgement.

The `risk` label states how much human scrutiny a change needs, not how much an automated review is
worth. `risk:high` means the change substantially affects the public API surface of a core tier
crate and needs maintainer-level scrutiny; `risk:medium` means a behavioural change that does not
substantially alter a core public API; `risk:low` means a self-contained change with no cross-crate
behavioural effect. Two deterministic rules override the classifier: a `cargo-semver-checks`
incompatibility always produces `risk:high`, because that check runs against the `ironrdp` facade
and every incompatibility it reports is a core public API break, and a breaking change that only the
classifier suspects raises its own `low` verdict to `risk:medium` rather than lowering its
`medium` or `high` judgement.

Because risk measures human scrutiny, it does not decide whether a protocol change is worth
reviewing: a `protocol_related` classification always earns an automated review, at any risk level.
For everything else, `risk:low` without `breaking-change` skips the review. Duplicates, `size/XL`,
a legitimacy stop, and the terminal review count still stop every route.

A `size/XL` pull request, meaning 800 or more changed source lines, is excluded from automated
review deterministically, before any model runs. The workflow comments once to explain the
exclusion and to point at [stacked pull requests](https://docs.github.com/en/pull-requests/get-started/about-stacked-prs)
for splitting dependent work, noting that stacks require every branch to live in this repository so
fork authors should open separate pull requests. The comment is removed automatically once a later
push brings the change below the threshold.

Run **Bootstrap pull request automation labels** once before enabling the workflow. It creates the
six labels defined in `.github/pr-automation/labels.json`. Existing labels for documentation,
duplicates, breaking changes, technical debt, SSPI, and PR size are reused.

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
