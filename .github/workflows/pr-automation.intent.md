## Concurrency model

Automatic classification and review use their canonical check run as a bounded ownership lease.
The marker contains only `v1`, the work kind, the exact head SHA, and the workflow run ID and attempt.
An active owner blocks queued or concurrent duplicate provider work, while a terminal, canceled, or missing owner permits a new claim.
This intentionally provides at-least-once model execution rather than artifact recovery, receipts, a transaction ledger, or lifecycle reconciliation.
The final writer verifies its ownership again, and canonical success remains absorbing for the same head.
Rare partial publication failures require manual repair rather than recovery automation.

The Helmcode key permits at most 25 parallel requests.

Allocate more capacity to slower review pipelines than to fast classifiers:

- Run at most 4 classifier agents in parallel.
- Run at most 7 reviewer pipelines, each with at most 3 specialist reviews in parallel.

That totals at most 25 parallel requests at any time: 4 for classifiers and 21 for review pipelines.
All model-output schemas are internal and unversioned.

For simplicity, derive static concurrency lanes from the pull request number instead of using an external semaphore service.

```text
classifier-lane = (pr-number % 4) + 1 // 1 through 4
review-pipeline-lane = (pr-number % 7) + 1 // 1 through 7
```

At the classifier job level:

```yaml
concurrency:
  group: llm-classifier-${{ classifier-lane }}
  cancel-in-progress: false
  queue: max
```

And for the reviewer pipeline job:

```yaml
concurrency:
  group: llm-reviewer-pipeline-${{ review-pipeline-lane }}
  cancel-in-progress: false
  queue: max
```

Each reviewer-pipeline lane must lock the entire review pipeline, not each reviewer job.
The review pipeline must therefore live in a reusable workflow, with lane concurrency on its caller job.

## Classification

- Configure the classifier action for at most four request retries after the initial attempt.
- Bound the streamed classifier stage independently of its enclosing job's cleanup allowance.

## Reviewer pipeline

- Require valid classification for the exact PR head; missing, stale, or invalid classification is an invocation error.
- Select specialists and identify which are required from that classification, then call `review-pipeline.yml`.
- Publish only after all required specialists succeed and the general review passes validation.

The published comments include the name of the specialist that found the finding.
Render severity as `critical :purple_circle:`, `high :red_circle:`, `medium :orange_circle:`, or `low :yellow_circle:`.
Append `:question:` for questions, and show `:green_circle:` in the main comment when no findings are found.
Disclose reduced coverage from optional reviewer failures in the published review and review check, naming each failed reviewer.
Keep detailed failure reasons in the workflow summary only.

### Stage execution

- Retry transient provider failures only within the logical model call that failed.
- Never restart a reviewer stage or rerun a completed stage.
- Set the 130-minute reviewer job timeout to cover one stage and cleanup.
- Keep all eligibility checks, resource limits, and stale-head protections in effect.
- Never publish the same review twice, and count only published reviews toward the two-review limit.
- Show the pipeline-reported outcome and LLM-stage metrics including unavailable usage in the review check and workflow summary.
- Link to the summary from the `AI automated review` check; keep metrics out of review comments.

## Activation policy

Classify every non-draft, human-authored pull request that passes the integrity and capacity gates.
Run automated review after the latest CI generation succeeds for the exact classified head.
Run the second review after a later push reaches green exact-head CI.
At `ai-reviewed/2`, the review pipeline stops and lifecycle reconciliation hands a green pull request to `needs-review`.

`needs-review` means a human reviewer is the next actor, and `needs-author-action` means the author is the next actor.
Lifecycle reconciliation exclusively owns these mutually exclusive actor labels.
An attempted exact-head classification or eligible review failure applies only `automation-failed`.
Every successful app-owned exact-head review check persists a bounded versioned `findings` or `no-findings` receipt.
Reconciliation trusts only a newest canonical successful check with the expected app, SHA, external ID, conclusion, schema, and exact receipt keys.
Existing successful checks without that receipt fail closed before lifecycle clearing.
Successful review outcomes, clean terminal handoffs, and legitimacy stops become actor labels only through a fresh reconciliation snapshot.
Closed and draft pull requests, active leases, and missing, pending, or nonstandard terminal CI conclusions select neither actor.
Exact-head CI failure selects `needs-author-action` independently of `automation-failed`.
The final writer accepts successful non-forced review and handoff state only when the stored CI generation is still the latest green exact-head CI generation.
The CI event is authoritative for its exact-head generation while listings catch up, and a newer listed exact-head generation always wins.

Every non-bot author is eligible immediately; there is no prior-merge requirement.

Block model review for likely non-legitimate changes and hand the green exact head to a human reviewer.
Label a suspected overlap with another pull request at confidence 0.85 or greater as `triage/overlap`, and keep it advisory: it never blocks review and never asks for maintainer handoff on its own.
Unavailable or invalid exact-head classification applies only `automation-failed`.
Risk and protocol relevance select reviewers but do not suppress review.

Fork pull requests share a quota of 50 per UTC day.
Exclude `OWNER` and `MEMBER` pull requests from enforcement and counting, and preserve the same-repository exemption.

Keep `size/XXL` informational.
Use a 1 MiB evidence diff limit by default and a 4 MiB limit when `ai-review/allow-oversized` is present.
Adding that label must retry classification.
Evidence above the applicable limit fails closed without partial model input.

Normal classification-to-review dispatch is edge-triggered and occurs only when the persisted SHA-bound classification check changes.
Adding `ai-review/allow-oversized` forces reclassification and may dispatch on the same SHA when the resulting classification state is unchanged.
Unrelated label events and repeated non-explicit unchanged classifications must not dispatch.

Force mode bypasses policy gates but not classification prerequisites, evidence, validation, filesystem, citation, publication, or stale-head safeguards.

Lifecycle events include ready and draft transitions, closure, relevant labels, and requested, in-progress, and completed CI states.
Automation-written label events may reconcile lifecycle but never start automatic classification on their own.
Classification admission, review admission, result publication, and reconciliation use one per-pull-request mutation boundary.
The workflow and mutation boundary retain queued wake-ups with non-canceling `queue: max` concurrency, so stale work revalidates and no-ops without interrupting active publication.
An admitted classifier that reaches draft publishes nothing.
An admitted review may publish its validated exact-head review, successful outcome check, and single count label while draft or closed-unmerged when all authority, CI, policy, and lease guards still pass.
Closed and merged pull requests never admit new automatic work, and merged pull requests publish no in-flight result.
If review creation is rejected after closed-unmerged validation, complete the lease neutral without a partial publication.

## Run summary

Link the resolved pull request from the workflow run summary, on every route.
