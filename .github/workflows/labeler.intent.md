## Concurrency model

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

## Reviewer pipeline

- Require valid classification for the exact PR head; missing, stale, or invalid classification is an invocation error.
- Select specialists and identify which are required from that classification, then call `review-pipeline.yml`.
- Publish only after all required specialists succeed and the general review passes validation.

The published comments include the name of the specialist that found the finding.
Render severity as `critical :purple_circle:`, `high :red_circle:`, `medium :orange_circle:`, or `low :yellow_circle:`.
Append `:question:` for questions, and show `:green_circle:` in the main comment when no findings are found.
Disclose reduced coverage from optional reviewer failures in the published review and review check, naming each failed reviewer.
Keep detailed failure reasons in the workflow summary only.

### Stage recovery

- Start a bounded pipeline that recovers transient stages within its invocation while retaining successful results.
- A later workflow run starts a fresh recovery budget.
- Keep all eligibility checks, resource limits, and stale-head protections in effect.
- Never publish the same review twice, and count only published reviews toward the two-review limit.
- Show the pipeline-reported recovery outcome and LLM-stage metrics including unavailable usage in the review check and workflow summary.
- Link to the summary from the `AI automated review` check; keep metrics out of review comments.

## Activation policy

Classify every non-draft, human-authored pull request that passes the integrity and capacity gates.
Run automated review after CI succeeds for the exact classified head.
Run the second review after a later push reaches green exact-head CI.
At `ai-reviewed/2`, the review pipeline stops; classification and its labels keep updating.

`OWNER` and `MEMBER` authors are always eligible.
Other authors need one pull request from the same immutable human author merged into `master`.

Block review for duplicates at confidence 0.85 or greater and likely non-legitimate changes.
Unavailable or invalid classification fails closed to maintainer review.
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
