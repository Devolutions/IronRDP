## Concurrency model

We’re using Helmcode for the automated classifier and reviewer pipeline.
We get at most 5 parallel requests.

To use those requests efficiently:

- Run at most two classifier agents in parallel.
- Run at most three reviewer agents in parallel.

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

Map specialists onto three fixed global lanes:

```yaml
concurrency:
  group: llm-reviewer-${{ reviewer-lane }}
  cancel-in-progress: false
  queue: max
```

To avoid relying on environment-secret propagation across reusable workflows, every job that reads `HELMCODE_GLM_API_KEY` is a normal job of `.github/workflows/labeler.yml` that binds `environment: llm-providers` itself.
The protocol and skeptical specialists each have a lane.
The code-compressor and general reviewers share the third lane because the general reviewer starts only after its own specialists finish.

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

They run in parallel as a multi-job matrix with at most three active entries.

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

Force mode bypasses policy gates but not evidence, validation, filesystem, citation, publication, or stale-head safeguards.
