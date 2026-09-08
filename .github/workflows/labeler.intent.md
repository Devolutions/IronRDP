## Concurrency model

We’re using Helmcode for the automated classifier and reviewer pipeline.
We get at most 5 parallel requests.

To use the 5 parallel requests as efficiently as possible:

- Run at most two classifier agents in parallel.
- Run at most one reviewer pipeline with at most 3 specialist reviews in parallel.

That totals at most 5 parallel requests at any time.
All model-output schemas are internal and unversioned.

For simplicity, derive static concurrency lanes from the pull request number instead of using an external semaphore service.

```text
classifier-lane = (pr-number % 2) + 1 // = 1 or 2
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
  group: llm-reviewer-pipeline
  cancel-in-progress: false
  queue: max
```

`llm-reviewer-pipeline` group must lock the entire review pipeline, not each reviewer job.
The review pipeline must therefore live in a reusable workflow, with `llm-reviewer-pipeline` concurrency on its caller job.

## Reviewer pipeline

- Select specialists, identify which are required, and call `review-pipeline.yml`.
- Publish only after all required specialists succeed and the general review passes validation.

The published comments include the name of the specialist that found the finding.
Render severity as `critical :purple_circle:`, `high :red_circle:`, `medium :orange_circle:`, or `low :yellow_circle:`.
Append `:question:` for questions, and show `:green_circle:` in the main comment when no findings are found.

### Stage recovery

- Schedule stage recovery for temporary failures, with delays and limits, without waiting for another PR or CI event.
- Keep all eligibility checks, resource limits, and stale-head protections in effect.
- Never publish the same review twice, and count only published reviews toward the two-review limit.
- Show whether stage recovery is pending, successful, or exhausted in the review check and workflow summary, with reasons for every stage failure.
- Show per-stage review metrics and totals across attempts in the workflow summary, including failed runs and unavailable data.
- Link to the summary from the `AI automated review` check; keep metrics out of review comments.

## Activation policy

Classify every non-draft, human-authored pull request that passes the integrity and capacity gates.
Run automated review after CI succeeds for the exact classified head.
Run the second review after a later push reaches green exact-head CI, and stop automatic review at `ai-reviewed/2`.

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

Force mode bypasses policy gates but not evidence, validation, filesystem, citation, publication, or stale-head safeguards.
