## Terms

- **Request retry:** resend the same provider request after a transient failure, without changing the conversation.
- **Output repair:** ask the model to correct an invalid response using validation feedback and the existing investigation context.

## Inputs

- Provider connection details and credentials.
- Model.
- Instructions.
- Output schema.
- Read-only access rules.
- Execution limits.
- Optional trusted validator for task-specific checks.

## Outputs

- JSON accepted by the schema and any supplied validator, or an explicit failure reason.

Keep diagnostic output bounded:

- Phase.
- Latency.
- Request-retry count.
- Output-repair count.
- Finish reason.
- Token usage when available.
- Accumulated turn and tool-call counts, including on failure.

## Request retries

- Retry transient provider failures with backoff while preserving investigation state.
- Do not automatically retry invalid configuration, rejected credentials, or exhausted quota.
- Keep request timeouts configurable and bound total time spent on request retries and output repair.
- Allow at most four retries per request after the initial attempt, including SDK retries.
- Count request retries separately from output-repair attempts, including retries of repair requests.

For retryable `429` responses:

- Read the provider's `Retry-After` header.
- Wait that duration, or fall back to backoff if absent.

Prefer the OpenAI SDK for `Retry-After` handling and request retries (`maxRetries`) where it satisfies the policy above.

## Output validation and repair

- Use provider-enforced schema output where supported, otherwise JSON mode where supported.
- Accept validators only from trusted caller configuration, never from untrusted evidence or model output.
- Validate JSON and schema locally, then run the supplied validator.
- Repair JSON, schema, and validator-reported output errors within the same invocation.
- Preserve the original investigation context during repair.
- Permit only necessary read-only evidence lookup during repair.
- Bound repair attempts and return failure when valid output cannot be produced within those limits.
