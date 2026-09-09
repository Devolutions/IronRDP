## Terms

- **Request retry:** resend the same provider request after a transient failure, without changing the conversation.
- **Output repair:** ask the model to correct an invalid response using validation feedback and the existing investigation context.

## Inputs

- Provider connection details and credentials.
- Model.
- Instructions.
- Output schema.
- Read-only access rules.
- Request timeout.
- Maximum request retries.
- Maximum model turns.
- Maximum tool calls.
- Maximum output size.
- Maximum output-repair attempts.
- Optional validator for task-specific checks.

## Outputs

- JSON accepted by the schema and any supplied validator, or an explicit failure reason.

Expose these diagnostics:

- Activity (such as investigation, final output, or repair).
- Duration of each provider attempt.
- Request-retry count.
- Output-repair count.
- Provider-reported stop reason (such as completion or token limit).
- Token usage when available.
- Accumulated turn and tool-call counts, including on failure.
- Each rejected output attempt, naming which validation rejected it and why.

## Request retries

- Retry transient provider failures with backoff.
- Do not automatically retry invalid configuration, rejected credentials, or exhausted quota.
- Apply the configured retry limit per request after the initial attempt.

For retryable `429` responses:

- Read the provider's `Retry-After` header.
- Wait that duration, or fall back to backoff if absent.

Prefer the OpenAI SDK for `Retry-After` handling and request retries (`maxRetries`) where it satisfies the policy above.

## Output validation and repair

- Use provider-enforced schema output where supported, otherwise JSON mode where supported.
- Accept validators only from trusted caller configuration, never from untrusted evidence or model output.
- Validate JSON and schema locally, then run the supplied validator.
- Repair JSON, schema, and validator-reported output errors within the same invocation.
- Permit only necessary read-only evidence lookup during repair, then ask for the corrected value without tools so it is produced under the configured output format.
- Report every rejected attempt with the validation that rejected it and a bounded, sanitized reason, and carry the last one in the failure reason.
- Give the validator every candidate parsed so far, oldest first, so a value the model added while repairing can be protected like one it opened with.
- Return failure if output remains invalid after the configured repair attempts.

Local schema and validator acceptance is always the authority.
Provider-enforced schema output constrains what the model returns; it never widens what this action accepts.
Support for it is per-model and per-schema, so a configuration selects it only where it is known to hold.
