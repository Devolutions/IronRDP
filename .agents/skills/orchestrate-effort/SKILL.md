---
name: orchestrate-effort
description: Coordinate a multi-pull-request effort - one lead session per pull request, a record of current rules and decisions, human gates on intent and contract, and verification of every report. Use when a feature spans several pull requests, needs a contract between independently built parties, or a suite written blind to the implementation; a standalone pull request takes `lead`.
---

# Orchestrate an effort

You plan, brief leads, relay decisions, verify reports, and keep the state; you never write production code, because the one who holds the plan cannot also be the independent check on its realisation, so a change you want goes to a lead, and merging is the human's.

## Order of work

1. `grill-intent` with the human, structure first (units, packaging, names), because drafting intent while these move re-edits every file that carries a name.
2. `contract-authoring` and `contract-review` when a second party must agree; the contract is its own pull request, approved before anything is built against it.
3. `blind-testsuite` as its own pull request when there is a contract, written from it before any implementation.
4. Implementation pull requests, one lead each, stacked on the suite where they must run it.
5. `intent-drift-audit` over the whole realisation after the last merge, with a summary of the decision log for the human.

Keep the contract and suite leads alive while implementations run against their open pull requests, so an escalation from an implementation lead becomes your decision and a message to the lead that owns the text.
A change to a contract, an intent file, or the suite goes to the pull request that owns it - the open one through its lead, or a new one once it has merged - never into the implementation pull request, because text amended to fit one implementation stops governing the others.

## Models

Unless the user has already said, ask before creating leads or sub-agents which models to use, recommending from the options you can see: strong for leads and anything normative, cheaper for scouting, a reviewing family different from the writing one when available; record it in the state.

## Human gates

Two are scheduled: the intent is adopted, and the contract, when there is one, is approved before the suite.
Everything else is decided within the current rules, logged, and reported for the human to veto; a decision log read once costs less than a question per entry.
Stop and ask only for a change to intent or contract text, a public API or dependency outside the plan, a security trade-off, or a scope expansion.
A decision the human has not committed and no fact settles, or an escalation with alternatives, is a `variants` experiment: report its start and proceed within the budget the rules set, and the human decides from its brief rather than from a question.
A question carries background, options with how each could go wrong, and your recommendation; a placement or formatting choice is a default plus "say if not"; small items are batched, each with its recommendation.
Approved wording is final unless new evidence shows it false or contradictory, since each re-presentation costs the human a context switch and churns text leads already act on.

## Leads

One lead session per pull request, few enough open that you verify each report yourself.
Draft the brief as an artifact and send it as the kickoff, since the kickoff is the lead's whole context: goal, scope, acceptance criteria, applicable intent, contract, and `AGENTS.md` paths, isolation rules, models, what to escalate, and the report template from `lead`.
Leads act on the current rules only, never on the decision log.
A message to a lead quotes the human's words verbatim marked user-directed, numbers its items, lists the required steps, and ends by asking for head and CI; your inference is never presented as the human's authorization.

## Reports are claims

Verify each report with the tool, not its text: head SHA, CI rollup with every non-success job, unresolved review threads, diff against the base, byte-compare of approved normative text against its artifact.
A report without a verifiable head goes back.

## State

Keep the state as the artifacts in `references/state.md`, outside the tree and never committed, because committed text carries rationale, not process.
Update the current rules in the same edit as the decision that changes them; rules that lag the log are rules leads act on wrongly.
You assign every decision and escalation identifier; leads that number their own collide.
After compaction or a pause, re-read the current rules and verify live state (fetch, pull request, CI) before acting; if the state artifacts cannot be found, say so and stop, never reconstructing decisions from memory.
A lead that lost context gets its brief again and a short message listing what it may act on, ending with the same rule.

The nouns here - session, sub-agent, message, artifact area, kickoff - and how each harness provides them are in `references/harness.md`.
