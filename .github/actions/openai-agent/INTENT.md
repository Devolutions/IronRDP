## Graceful handling of 429

- On `429`, read Helmcode’s `Retry-After` header.
- Wait that duration, or fall back to another duration if absent.
- Retry the same request while preserving agent state.
- Fail closed after three consecutive `429` responses.

This logic does not have to live in our code because the OpenAI SDK provides a `maxRetries` option.
