# Purpose and trust boundary

This action runs a bounded filesystem agent against an OpenAI-compatible chat-completions API.
Workflow configuration, prompts, output schemas, and optional validator modules are trusted.
Repository evidence, tool arguments, and provider responses are untrusted.
The model receives only configured read-only filesystem capabilities.
The action has no general write capability.

# Execution and recovery

- A **logical model call** sends one message history to the provider.
- An **HTTP attempt** is one network request for a logical model call.
- A **request retry** resends the unchanged logical call after a transient failure.
- An **output repair** asks the model to correct a rejected value within the existing conversation.

Every invocation has finite turn, tool-call, repair, and stage wall-clock budgets.
Every HTTP attempt has finite streamed-byte and idle-progress budgets.
The stage deadline uses monotonic time and covers provider attempts, retry waits, tools, validation, and repairs.
The idle deadline starts before the first response byte and resets only when the response body makes progress.
The action retries transient failures within the logical call's request budget and remaining stage time.
Retries discard all fragments from the failed attempt and preserve the unchanged message history.
Provider-directed retry delays and fallback backoff consume the same stage budget.
Configuration errors, rejected credentials, exhausted quota, invalid output, and exhausted limits are terminal.

The action assembles each streamed response completely before using it.
Content, reasoning, usage, finish reason, and indexed tool-call fragments are accumulated independently.
Missing optional reasoning or usage metadata does not invalidate otherwise accepted text.
Every accepted stream carries a terminal finish reason so cleanly truncated responses are retried rather than accepted.
Tool calls execute only after the stream ends cleanly, the provider reports a tool-call finish, every envelope is complete, and the whole batch fits the remaining tool budget.
No tool from an incomplete or malformed batch executes.

# Output acceptance and repair

Raw model content is bounded before it enters local JSON parsing or message history.
Checked-in schemas bound retained text and collections, while runtime limits independently bound raw streams and accepted serialization.
Unpaired UTF-16 surrogates are replaced with the Unicode replacement character before validation.
Sanitization rejects object keys that would become ambiguous after replacement.

Provider output constraints are generation aids selected only for model and schema combinations known to support them.
They never widen local acceptance.
They may be narrower than local acceptance when a trusted normalizer can preserve useful work without ambiguity.
Local JSON parsing, schema validation, and the trusted semantic validator are authoritative.
Local acceptance preserves useful work by normalizing unambiguous representation defects before applying downstream constraints.
Missing provider diagnostics do not narrow that local acceptance.

Rejected JSON, schema, and semantic output may be repaired within the configured budget.
Repair may read only necessary evidence and must preserve every usable finding from earlier candidates.
Every rejection records its validation layer and a bounded, sanitized reason.
The action fails when the repair budget ends without an accepted value.

# Output transport

Accepted JSON is returned as a step output unless the caller supplies a workflow-controlled output-file target.
An output-file target is input only, and the action outcome reports whether the write succeeded.
File transport is confined to a new regular JSON file in the action's dedicated workspace directory.
The action rejects existing targets, symbolic links, symbolic-link directories, and paths outside that directory.

# Observability

Diagnostics identify logical calls and physical attempts without exposing prompts, model content, credentials, or raw provider errors.
They retain bounded request sizes, tool-result sizes, durations, retry and repair counts, provider stop and error codes, token usage when available, and validation rejections.
Turn and tool counts remain available on failure.
