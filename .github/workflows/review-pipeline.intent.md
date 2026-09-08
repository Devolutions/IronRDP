## Reviewer pipeline

`review-pipeline.yml` is a reusable GitHub Actions workflow with `workflow_call` as its only trigger.
This lets specialists run in parallel while the caller applies a global concurrency limit to the entire pipeline.
The caller owns publication.

```text
evidence -> specialist reviewers running in parallel -> general reviewer aggregating everything
```

The general reviewer independently inspects the pull request, attempts to falsify every candidate, and records exactly one `accepted`, `refined`, or `rejected` disposition per candidate.
It can merge overlapping candidates and add findings that no specialist reported.
Reviewer findings use severity and a question boolean.

- Configure reviewer actions for at most four request retries after the initial attempt.

### Specialist reviewers

The caller selects specialists and identifies which are required.
Supported specialists include:

- protocol
- skeptical
- code compressor

Run selected specialists in a multi-job matrix, with at most three running at once.

### Stage recovery

Stage recovery happens within one workflow invocation and keeps the results of stages that already succeeded.

- Retry a stage once, after a delay, when the provider fails transiently.
- Recheck the head and review prerequisites before a delayed retry.
- Supply a review-specific validator and let the action handle bounded output repair.
- Never discard findings during output repair; fail the stage if repair cannot produce valid output.

Reusing results across workflow runs is not required.
A later run starts fresh.

### Outputs

- Return a validated general review only when every required specialist succeeds.
- Return every failed stage and its reason to the caller.

Return per-stage metrics, including failed attempts and marking unavailable data:

- Token usage.
- Elapsed time.
- Request-retry count.
- Output-repair count.
- Stage-recovery count.
- Whether results were reused.

Do not count reused results as new token usage.
