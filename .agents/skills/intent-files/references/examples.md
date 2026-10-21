# Examples the maintainer wrote

Only files the maintainer has confirmed as examples appear here.
Read the file at the path given; what follows is only what makes it work.

## `xtask/INTENT.md` (Devolutions/devolutions-gateway, branch `cbenoit-build-xtask-checks`, unmerged at the time of writing)

Until that branch merges the file is not on the default branch, and the description below stands on its own; the path is updated when it lands.
Written by hand for a new crate and committed as written, about 100 sentences.
No title; `## Overview` holds the purpose in one line, then non-goals ("The intention is not to rewrite ci.yml in Rust", "Do not preserve tlk.ps1 as an abstraction") and deferrals ("For now, ignore Windows installers", "remains a CI-only check for now") side by side, each one sentence.
`## Dependencies` is a four-crate allowlist plus a goal ("compile under one second"): an inventory that is itself the decision.
`## Pipeline` pins the stage order as an arrow diagram and numbered steps, and calls out the one ordering that matters (`canonicalize -> deduplicate -> apply declared coverage -> merge -> order`): implementation shape, deliberately chosen.
`## CLI` gives the literal usage text and the selector semantics in four sentences ("Repeated check names form a union", "A selector that unexpectedly matches nothing is an error"): an exact contract inline, because nothing implements this interface independently.
`## Concepts` is a glossary, one `###` per term, one to three sentences with an example; the rules live inside the definitions ("A missing requirement is an error, not an applicability skip", "Prerequisites must not be used to install missing requirements implicitly") and `NOTE:` marks a future direction or an anti-requirement.
Voice mixes imperative and declarative; under the one-meaning-per-verb rule its "`cargo xtask` should remain a developer-focused tool" reads as a recommendation, and no sentence is about agents.
Beside it, `xtask/AGENTS.md` is seven lines of module-organisation policy for contributors ("Prefer vertical task modules", "Do not add or modify dependencies without explicit approval") and nothing else, and `xtask/EVIDENCE.md` maps each design claim to the directive, intent line, and code path that realise it, with `[unsure]` on inferred rows.

The same author's `xtask/src/model.intent.md` shows the one thing to avoid: firm wants (the exact `Subject` variants; permission to use `cargo metadata` for discovery) written as "subjects I have in mind" and "may be used" because their placement was unclear.
The set belongs under `### Subject` as "exactly these", the permission as "discovery is permitted to use `cargo metadata`", and the placement doubt in the PR.
