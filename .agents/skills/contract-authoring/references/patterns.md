# Patterns from two agent-written contracts

`docs/agent-identity/CONTRACT.agent-identity.md` (Devolutions/devolutions-gateway, PR #2000) and `picky-crypto/CONTRACT.md` (Devolutions/picky-rs) were written by agents under maintainer review.
They are not shape references; use them to recognise what review kept and what it had to fix.

## Kept by review

- General rules that apply to every operation, stated once at the top with "these rules are not repeated", so per-operation sections stay short.
- Precedence as which outcome wins, not an evaluation order: "a bounds error always wins; a capacity limit may win over a structure error; this fixes which outcome wins, not the order among a backend's checks".
- Implementation-defined outcomes as closed sets per input: "loading returns `InvalidKey` or succeeds; signing returns `InvalidKey` or a signature that verifies under (n, e)".
- Stored-field ownership as one sentence naming the operations: "renewal, confirm and check-in update `last_seen_at`; an idempotent enrollment replay and stream closure do not".
- The error section as the only list of codes, mappings, and retry rules.
- Exceptions inline with the rule they relax.

## Fixed by review, so look for them

- Claims that overreached and were narrowed: "prevents oracles" became "does not reveal which check failed"; "cannot be compared" became "cannot be compared directly".
- One library's behaviour as a general rule: "p = q" for an inconsistent RSA key.
- A number and a rationale tied to one backend, with the cause never measured.
- A second copy of a list: the deletion list missed `device_revoked`.
- An example standing in for a rule: signature parameter order was shown, not stated, so a conforming library reproduced no vector.
- Two encodings for one concept: hex and base64url for the same hash.
- Lost-response, deleted-in-between, concurrency, and expiry-mid-flow paths unspecified until a critique asked.
- A verifier accepting the client's own list of covered components.
- A product's admin API and the mock's control surface inside the contract; both are product or test surface, and each belongs in the intent of the component that implements it.
