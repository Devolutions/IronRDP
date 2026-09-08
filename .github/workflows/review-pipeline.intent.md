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
- Produce schema-conforming output with minimal, bounded repair and review-specific semantic validation.
- Report each rejected output attempt's validation reason with bounded, sanitized diagnostics.
- Never discard findings during output repair; fail the stage if repair cannot produce valid output.

Reusing results across workflow runs is not required.
A later run starts fresh.

### Outputs

- Return a validated general review only when every required specialist succeeds.
- Return every stage's outcome and failure reason, distinguishing reduced coverage from full completion.

Return metrics only for LLM-powered stages, including failed attempts and marking unavailable data:

- Clearly labeled input, output, and total token usage, accumulated across requests.
- Elapsed time in seconds; label summed durations as cumulative rather than wall-clock time.
- Request-retry count.
- Output-repair count.
- Stage-recovery count.
- Which stages repeated during recovery.

Count each provider attempt only once.
