## Concurrency model

We’re using Helmcode for the automated classifier and reviewer pipeline.
We get at most 5 parallel requests.

In order to use the 5 parallel requests as efficiently as possible:

- Run at most two classifier agents in parallel.
- Run at most one reviewer pipeline with at most 3 specialist reviews in parallel.

All in all, we get at most 5 parallel requests at any time.

For simplicity, use static concurrency lanes using the pull request number (as opposed to using an external semaphore service).

```
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

```yml
concurrency:
  group: llm-reviewer-pipeline
  cancel-in-progress: false
  queue: max
```

`llm-reviewer-pipeline` group must lock the entire review pipeline, not each individual reviewer job.
To that end, the review pipeline must live in a reusable workflow, and the `llm-reviewer-pipeline` concurrency is placed on its caller job.

## Reviewer pipeline

The reviewer pipeline is roughly defined as:

```
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
As discussed in the concurrency model section, we allow at most three specialist agents to run at once.
