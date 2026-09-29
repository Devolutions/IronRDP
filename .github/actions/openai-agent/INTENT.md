# OpenAI filesystem agent intent

This action runs a narrowly capable, bounded filesystem agent against an OpenAI-compatible chat-completions API.
It exists to let trusted workflows use model reasoning and read-only evidence lookup without granting command execution, arbitrary network access, or general filesystem writes.
Repository evidence, tool arguments, and provider responses remain untrusted throughout the invocation.

## Terms

- A **logical model call** sends one message history to the provider.
- An **HTTP attempt** is one network request for a logical model call.
- A **request retry** resends the unchanged logical call after a transient failure.
- An **output repair** asks the model to correct a rejected value using validation feedback and the existing conversation.

## Trusted inputs and capabilities

The workflow controls:

- Provider connection details and credentials.
- Model and instructions.
- Output schema.
- Read-only filesystem capabilities.
- Stage, stream-idle, request-retry, model-turn, tool-call, and output-repair limits.
- An optional task-specific validator and its bounded metadata.
- An optional structured-output file target.

Configuration, prompts, schemas, and validator modules are trusted workflow inputs.
Evidence and model-produced tool arguments never grant capabilities.
The model can call only the declared `read_file`, `list_files`, and `search_text` tools within the configured paths and resource limits.

## Streaming and stage limits

Every model call uses streaming so active reasoning and generation are not bounded by a short whole-request timeout.
The action requests high reasoning effort and leaves the provider token cap unset.

The stage has one monotonic deadline covering provider attempts, retry waits, tool execution, validation, and output repair.
Every HTTP attempt has independent raw-response-byte and idle-progress limits.
The idle limit begins before the first response byte and resets only when the response body makes progress.
The raw-stream limit includes framing bytes before the SDK parses server-sent events.

The action accumulates reasoning, content, usage, finish reason, and indexed tool-call fragments independently.
Truthy indexed tool identity snapshots (`id`, `type`, and `function.name`) replace earlier values, while `function.arguments` appends in arrival order.
Reasoning remains separate from tool-call accumulation.
An omitted choice index is accepted only for a single-choice chunk, and one complete index-less tool-call envelope is accepted only when the response contains no indexed fragments.
Every accepted stream carries a terminal finish reason so cleanly truncated responses are retried rather than accepted.
After that terminal reason, the conventional `choices: []` usage-only tail is preserved unchanged.
The only accepted nonempty tail is one choice with an empty delta and an omitted, null, or matching finish reason, which maintains compatibility with providers that include usage metadata there.
Every other nonempty post-finish tail remains a structural violation.
Missing optional reasoning or usage metadata makes diagnostics incomplete but does not invalidate otherwise acceptable text.

Tool calls execute only after the complete stream has arrived, the provider reports a tool-call finish, every envelope is structurally valid, and the whole batch fits the remaining tool budget.
No tool from an incomplete or malformed batch executes.

## Request retries

One model turn is one logical model call regardless of its number of HTTP attempts.
The action retries only transient provider failures and never retries invalid configuration, rejected credentials, exhausted quota, invalid model output, or exhausted local limits.
The configured request-retry limit applies after the initial HTTP attempt.
Retries reuse the unchanged message history and discard every fragment from the failed attempt.
An interrupted streamed body is safe to retry because no partial assistant message or tool call has been committed.

For retryable responses, the action honors a valid provider `Retry-After` value and otherwise uses bounded exponential backoff with jitter.
Retry delays and repeated attempts consume the same stage deadline.
A reviewer workflow does not restart the whole action after this per-call recovery is exhausted.

## Output acceptance and repair

The action returns JSON accepted by the local schema and any supplied validator, or an explicit bounded failure reason.
Raw model content is bounded before it enters local JSON parsing or message history.
Accepted serialization is bounded independently of the configured schema.
Checked-in schemas also bound their retained text and collections.

Unpaired UTF-16 surrogates are replaced with the Unicode replacement character before validation.
Sanitization rejects distinct object keys that would collide after replacement.

Provider JSON or JSON Schema constraints are generation aids selected only for model and schema combinations known to support them.
They may be narrower than local acceptance when a trusted normalizer can preserve useful work without ambiguity.
They never widen local acceptance.
Local JSON parsing, schema validation, and the trusted semantic validator remain authoritative.

The action repairs rejected JSON, schema, and semantic output within the configured repair budget and existing conversation.
A semantic repair may use only necessary read-only evidence lookup.
After evidence lookup, the corrected value is requested without tools so the configured provider output format applies where supported.
Every parsed candidate is supplied to the validator in original order so repair cannot silently discard usable findings.
Every rejected attempt records the validation layer and a bounded, sanitized reason.
The final rejection reason is retained when the repair budget is exhausted.

## Structured output files

Accepted JSON is returned as a step output unless the caller supplies a workflow-controlled output-file target.
The target is input only; the action step outcome reports whether generation, validation, and the write succeeded.
The target must name one new `.json` file below `.openai-agent-output` under the real workspace root.
The action rejects symbolic-link directories, symbolic links, existing targets, and paths outside that directory.
This exception transports accepted output without turning the action into a general write capability.

## Diagnostics

Expose these bounded diagnostics:

- Activity such as investigation, finalization, or repair.
- Deterministic logical-call and HTTP-attempt indices.
- Message count, serialized request bytes, and accumulated tool-result bytes for each logical call.
- Duration of each HTTP attempt and the full invocation.
- Request-retry and output-repair counts.
- Provider finish reason and bounded error code when available.
- Token usage and whether it is complete.
- Accumulated turn and tool-call counts, including on failure.
- Every rejected output attempt with its validation layer and sanitized reason.
- The first stream structural violation as a closed static value with no provider data.
- Ignored empty post-finish choices, repeated terminal-choice subsets, and the first rejected post-finish shape as closed, content-free values.

Diagnostics and logs never expose credentials, prompts, repository evidence, tool arguments, tool results, reasoning, model content, or raw provider errors.
Stream structural diagnostics use only a closed static vocabulary.
