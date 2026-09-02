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
