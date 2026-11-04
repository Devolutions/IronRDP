---
name: contract-authoring
description: Author or extend a normative CONTRACT*.md - an exact, testable specification of an interface, protocol, or provider boundary that independent implementations and a conformance suite derive from. Use when designing a spec, contract, protocol document, or provider API that several components or backends must agree on.
---

A contract defines a surface that two independently built parties must agree on: a backend author implements from it alone, and the conformance suite derives its expectations from it and the standards it cites, never from an implementation.
It names the parties in scope without schedule ("e.g.: Product A, Product B", not "A now, B later"), since a release stage is not a rule a party agrees to, and a statement belongs only if one of them must agree to it.
Of a party's own realisation (`intent-files`), only the parts a named second party depends on enter.
`INTENT.md` beside it explains the choices; the contract states the rules.
Name the file `CONTRACT.<name>.md` when the repository may hold more than one contract, and cite it everywhere as `CONTRACT.<name>.md §<anchor>`, because the file name is the one citation that is unique, searchable, and readable without a lookup.

## Process

1. **Scout before writing.**
   Inventory every call site the contract will serve, with `file:line`.
   Read each reference implementation or host at a pinned version and record, per operation, whether it is native, argument mapping, workaround, or unsupported, with evidence.
   Host conventions (naming case, pagination, proxies that rewrite paths) become rules; a reference you did not read becomes a false universal.
2. **Settle the design by interview**, one frontier of decisions per round as in `grill-intent`; the human decides, and the agent may draft the normative wording for each decision, since a contract is agent-written under human approval.
   Number every clarification and log who decided it.
3. **Draft, then critique with two lenses**: overkill and gaps.
   Run every state-changing operation through `references/critique-lenses.md`.
4. **Freeze** once the design is approved.
   Afterwards it changes only for implementation defects, in its own PR ahead of the code that needs it, so the change is reviewed as a rule and not excused by the code it unblocks.
   Implementers never edit the contract or the test suite to make code pass; they escalate with evidence from the cited source.

## Content rules

- Per operation: the valid domain, the must-support set, the exact error for each class of invalid input from a closed error set, and the precedence of outcomes.
  Phrase precedence as which outcome wins, not as an evaluation order, since only the outcome is observable and an order binds a backend's internal checks.
- Implementation-defined cases are listed with their exact allowed outcome set, never with examples, and are excluded from differential tests.
- For every stored field: who initialises it, which operations update it, and which must not.
- Every list exists once; a second copy may drift.
- A `Definitions` section with one entry per term the parties must share, in the glossary shape from `intent-files`; one encoding per concept, and the same word for the same thing throughout.
- The contract is the single source for the API it specifies; when it covers a code API, public items cite their section and a check keeps them in step.
- Each operation and normative section has an anchor as `intent-files` Anchors defines, because the conformance suite cites every rule and the code and the intent cite the operations they realise.
- Detail beyond the intent is the contract's job, but when a contract rule needs a design decision the intent does not take, the human records that decision in `INTENT.md` first; a design decision made only in the contract is one nobody made.
- A contract has no versions and no deferral markers: it describes the whole surface as it must be, and the subset realised today is a deferral in the co-located intent file, extended by editing; a version negotiated on the wire is content.
- An example is not a rule: state what is free (order, case, whitespace) and add vectors that vary it, or a conforming library will reproduce none of them.
- Put the exception where the rule is: "terminal, except during pending-key recovery".
- Strict where it matters, extensible elsewhere: optional fields older receivers ignore, but authentication data stays strict, since an ignored field in authentication data is an unverified input.
- A derivable operation stays out unless a written exception (performance, validated-module boundary, a security property) sits next to it, since each one widens what every backend must implement and test for a result the primitives already give.
- Leave out one library's behaviour and numbers, admin and UI surfaces, mock and test-harness control surfaces (those are the intent of the component that implements them), implementer choices, and process.
- A rationale must be as true as its rule, since a reader who finds the cause false drops the rule with it; measure before citing a cause.

## Testability

Write each rule with one condition and one response ("when …", "while …", "if … then …") and one meaning per verb: declarative present for a requirement, `should` a recommendation, `may` a permission, `can` a capability.
Each rule maps to a conformance case, worded as an observable with a bound: "encode writes exactly `size()` bytes", never "handles" or "supports", which name no outcome a test could check and so let divergent implementations all comply.
"The backend supports RSA-PSS" is met by signing, by returning `Unsupported`, or by producing bytes nobody verifies; "signing with RSA-PSS returns a signature that verifies under the public key, or `Unsupported` before any key material is read" is one outcome and one test.
The contract's rules define conformance, and it has no conformance section, because defining conformance as passing a suite makes the evidence the rule, so a party that follows the contract stops conforming when the suite has a bug.
Test vectors and suites are evidence for the rules, so their role, their pass conditions, the mapping of rules to tests, the review-only claims, and the known gaps belong to the suite and the intent that governs it, never to the contract.

Beyond that, shape is free: order sections so a reader finds all of an operation's rules in one place.
`references/patterns.md` lists what review kept and what it fixed in two agent-written contracts; read it when revising one or checking a draft.
