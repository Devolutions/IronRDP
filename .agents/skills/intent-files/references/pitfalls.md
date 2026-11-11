# How intent drafts fail review

Each entry is a mistake that reached review, the correction, and the rule it taught; the maintainer's words are quoted where they exist.
Use it as the checklist when reviewing or before handing a draft back.

## Process dressed as intent

- "Adding a new `RecordingFileType` requires an explicit decision" constrains the contributor, not the software.
  The commitment is "a variant without an explicit `/shadow` mapping is rejected, never given a fallback streamer".
- Stages, decision IDs, agent roles, PR sequencing, and test-writing workflow appeared in drafts.
  "Just the spec, and implementation decisions, nothing on the agent operational side."
- A testsuite line referred to agents; the maintainer reworded it "so it no longer refers to agents". ([picky-rs #556](https://github.com/Devolutions/picky-rs/pull/556#discussion_r4238310499))

## Plan or ticket, not commitment

- "The intent file should be normative. 'We would like to...' only fits behavior we intentionally haven't implemented yet, and this behavior is already implemented." ([devolutions-gateway #1942](https://github.com/Devolutions/devolutions-gateway/pull/1942#discussion_r4157014381))
- "This sounds like a ticket body. Write in present tense, in a normative way, describing how things are expected to be at any point in time." ([#2003](https://github.com/Devolutions/devolutions-gateway/pull/2003#discussion_r4129433952))
- Firm wants written tentatively because their placement was unclear: "I'm not sure yet what is the correct place to document these, but here are subjects I have in mind" and "`cargo metadata` may be used".
  The set and the permission were requirements; state them as such, and put the placement doubt in the PR rather than the file.
- A future direction is allowed as a short, labelled "Later, X could …" note, or as a constraint ("V1 must not preclude this migration").

## Not true, or not true everywhere

- "'Each client begins at its own live edge' isn't always true with the current implementation. Can we state the actual rule here?" ([#1942](https://github.com/Devolutions/devolutions-gateway/pull/1942#discussion_r4198134333))
- "Every library imports SPKI": ring has no SPKI parser; it became "the contents of the SPKI `subjectPublicKey`".
- "Fill-in providers implement what no library offers": aws-lc offers the KDFs; it became "what some backend's library lacks".
- "The agent never sends its certificate to authenticate": a planned mTLS path sends it to another component; it became "to the authority".
- "The bundle's `fips()` is false": a bundle holding only an aws-lc FIPS build falsifies it; it became "a non-FIPS convenience that claims nothing".
- "It's 2026.3.0, specifically; because 2026.3.1 is also 2026.3 and it will have a different format." ([#2039](https://github.com/Devolutions/devolutions-gateway/pull/2039#discussion_r4221382200))
- "Either drop the backward-compatibility claim or narrow it to 'Clients receive no `SegmentStarted` message before the first segment'." ([#1942](https://github.com/Devolutions/devolutions-gateway/pull/1942#discussion_r4157014381))
- "A hand-written input never expects success" broke when boundary controls needed acceptance; it became three lines naming the exceptions.
- "`Unsupported` is allowed above the must-support set" missed inputs below it; it became "outside".
- "p = q" in an inconsistent-key rule was one RSA crate's behaviour; it became "components that are not a consistent RSA private key for (n, e)".
- A capacity-limit rationale cited stack growth during decoding; measurement showed key construction grew the stack.
  A reason must be as true as its rule.

## Belongs elsewhere, or nowhere

- A rationale intent restated the token format and operations table under "decisions it must preserve"; the maintainer made the contract self-contained and kept only the choices and reasons.
- "This is verbatim the same as what is written in the README.md. INTENT.md is not intended to be the same as README.md." ([IronVNC #1542](https://github.com/Devolutions/IronVNC/pull/1542#discussion_r4013943689))
- "I would remove this, as it's part of the file name / convention." ([#1946](https://github.com/Devolutions/devolutions-gateway/pull/1946#discussion_r3898220402))
- "A backend is a thin adapter" and "it passes the full conformance suite" in every backend intent drew "does that belong to this crate?" and "how useful is that invariant?"; both were cut.
- Scope / includes / excludes / "Component boundaries" / a closing "Boundary" note said the same thing four times in one file; one sentence next to the rule it qualifies would do.
- A table of file types with their extensions duplicated the mapping the same file said lived in code; the accepted-versus-rejected policy was the intent, the extension column was not.

## Enforced but unwritten

- "The actual wire contract isn't written down. Suggest rewriting it as: inputs, normalized output guarantees, and an exact Start/Pull/response transcript." ([#1942](https://github.com/Devolutions/devolutions-gateway/pull/1942#discussion_r4156764692))
- "`gateway-db` must never depend on crates that implement feature behavior. Adapters may depend on `gateway-db`, but never the other way around. Please add this to the invariants in `crates/gateway-db/INTENT.md`." ([#2039](https://github.com/Devolutions/devolutions-gateway/pull/2039#pullrequestreview-5457434157))
- "Write some human-vetted invariants about the concrete invariants and intentions you encoded when authoring this PR." ([#1946](https://github.com/Devolutions/devolutions-gateway/pull/1946#pullrequestreview-5018598898))

## Rule without its reason

- "`fips()` always returns `false`" drew "explain why"; it gained "because RustCrypto has no FIPS 140 validated module, so the FIPS policy rejects any provider containing its entries".
- "It's good to explain we don't want to turn that into a generic LLM client. Also, I would document that a vendored implementation is intentional to reduce dependencies." ([#2005](https://github.com/Devolutions/devolutions-gateway/pull/2005#discussion_r4156224738))

## Files that disagree

- "The two intent files now disagree. Both are human-owned, so the wording needs the maintainer's approval." ([picky-rs #572](https://github.com/Devolutions/picky-rs/pull/572#discussion_r4282323307))
- After "thin adapter" was dropped, "the thin-adapter rule applies to backends only" pointed at nothing; grep for every reference when removing a rule.
- Crate intents linked to a shared design file under `docs/`; renaming it broke four links, and a link confers no applicability under the convention.
  A shared design is an `INTENT.<feature>.md` part symlinked into each governed directory.

## Timing

- Packaging changed three times and each time the intents, the contract, and every file naming the crate changed; drafts written mid-design needed sixteen corrections a day later.
  Draft after decisions settle, and ship intent edits in a small PR ahead of the code that depends on them.
- Encodings, a thumbprint definition, and a grace-window formula were defined by the drafter and never discussed; track provenance and list filled gaps for the human.

## Form

- One sentence per line, typos listed exhaustively, trailing newline, no heading that repeats the folder name. ([#1942](https://github.com/Devolutions/devolutions-gateway/pull/1942#discussion_r4156764692), [IronVNC #1544](https://github.com/Devolutions/IronVNC/pull/1544#discussion_r4024059256))
