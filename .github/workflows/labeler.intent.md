## Concurrency model

We’re using Helmcode for the automated classifier and reviewer pipeline.
We get at most 5 parallel requests.

To use the 5 parallel requests as efficiently as possible:

- Run at most two classifier agents in parallel.
- Run at most one reviewer pipeline with at most 3 specialist reviews in parallel.

That totals at most 5 parallel requests at any time.

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

The reviewer pipeline is roughly defined as:

```text
evidence -> specialist reviewers running in parallel -> general reviewer aggregating everything
```

The general reviewer independently inspects the pull request, attempts to falsify every candidate, and records exactly one `accepted`, `refined`, or `rejected` disposition per candidate.
It can merge overlapping candidates and add findings that no specialist reported.
Only the validated general-review result can be published.
The published comments include the name of the specialist that found the finding.

### Specialist reviewers

Currently we define three specialist reviewers:

- protocol
- skeptical
- code compressor

They should run in parallel using a multi-job matrix.
We allow at most three specialist agents to run at once.

## Activation policy

Classify every non-draft, human-authored pull request that passes the integrity and capacity gates.
Run automated review after CI succeeds for the exact classified head.
Run the second review after a later push reaches green exact-head CI, and stop automatic review at `ai-reviewed/2`.

`OWNER` and `MEMBER` authors are always eligible.
Other authors need one merged pull request from the same immutable human author, excluding `trivial`, `reverted`, and revert-titled pull requests.

Block review for duplicates at confidence 0.85 or greater and likely non-legitimate changes.
Unavailable or invalid classification fails closed to maintainer review.
Risk and protocol relevance select reviewers but do not suppress review.

Fork pull requests share a quota of 50 per UTC day.
Exclude `OWNER` and `MEMBER` pull requests from enforcement and counting, and preserve the same-repository exemption.

Keep `size/XXL` informational.
Use a 1 MiB evidence diff limit by default and a 4 MiB limit when `ai-review/allow-oversized` is present.
Adding that label must retry classification.
Evidence above the applicable limit fails closed without partial model input.

Force mode bypasses policy gates but not evidence, validation, filesystem, citation, publication, or stale-head safeguards.
