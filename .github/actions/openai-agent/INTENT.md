## Inputs

- Provider connection details and credentials.
- Model.
- Instructions.
- Output schema.
- Read-only access rules.
- Execution limits.
- Optional caller-supplied validation errors for repairing a previous result.

## Outputs

- Schema-validated JSON on success, or an explicit failure reason.

Keep diagnostic output bounded:

- Phase.
- Latency.
- Retry and repair counts.
- Finish reason.
- Token usage when available.
- Accumulated turn and tool-call counts, including on failure.

## Bounded recovery

- Retry transient provider failures with backoff while preserving investigation state.
- Do not automatically retry invalid configuration, rejected credentials, or exhausted quota.
- Keep request timeouts configurable and bound total recovery time.
- Account for SDK retries within these bounds.

For retryable `429` responses:

- Read the provider's `Retry-After` header.
- Wait that duration, or fall back to backoff if absent.
- Fail closed after three consecutive `429` responses.

Prefer the OpenAI SDK for `Retry-After` handling and request retries (`maxRetries`).

## Output validation and repair

- Use provider-enforced schema output where supported, otherwise JSON mode where supported.
- Validate JSON and schema locally before returning success.
- Repair JSON and schema errors internally.
- Let callers request repair of a previous result using their task-specific validation errors.
- Preserve the original investigation context for both repair paths.
- Permit only necessary read-only evidence lookup during repair.
- Bound repair across follow-up requests and return failure when valid output cannot be produced within those limits.
