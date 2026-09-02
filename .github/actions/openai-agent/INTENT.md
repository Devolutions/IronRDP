## Graceful handling of 429

- On `429`, read Helmcode’s `Retry-After` header.
- Wait that duration, or fallback to another duration if absent.
- Retry the same request while preserving agent state.
- Fail closed if receiving `429` 3 times in a row.

This logic does not have to live in our code, the OpenAI SDK provides a `maxRetries` option which can be used.
