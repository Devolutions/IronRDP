## Inputs

- Provider connection details and credentials.
- Model.
- Instructions.
- Output schema.
- Read-only access rules.
- Execution limits.
- Optional validation feedback for output repair.

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
- On retryable `429`, read the provider's `Retry-After` header.
- Wait that duration, or fall back to backoff if absent.
- Retry the same request while preserving agent state.
- Fail closed after three consecutive `429` responses.

## Validated output

- Use provider-enforced schema output where supported, otherwise JSON mode where supported.
- Local schema validation is the real acceptance boundary.
- Use schema errors or caller-supplied validation errors for bounded output repair.
- Reuse investigation context and permit only necessary read-only evidence lookup.
