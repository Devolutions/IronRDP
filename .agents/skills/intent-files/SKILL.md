---
name: intent-files
description: Discipline for INTENT.md and *.intent.md files - the durable design decisions and rationale behind a component; what belongs there versus CONTRACT*.md or AGENTS.md, where they go, how to word statements so they stay true, and how cited sections keep stable anchors. Use when drafting or revising intent files, when anchoring a section for citation, or when another intent skill needs the vocabulary.
---

# Intent files

An intent file holds the design commitments a human has made durable: each one rules out a plausible, locally correct change that was deliberately rejected.
It is authoritative where present and optional everywhere; `AGENTS.md` constrains how contributors work, `CONTRACT.md` or `CONTRACT.<name>.md` exists only for a surface that two independently built parties must agree on.
A component's own interface (its CLI, config, storage) with the reason it is shaped so is intent, not contract, until a named second party depends on it.
If the repository has `docs/intent-files.md`, it is the house convention.

An agent never writes intent from code or a brief, and never creates, edits, or deletes an intent file on its own; it may interview the human (`grill-intent`), record the commitments the human states or adopts, tidy them to these conventions, and hand the assembled file back for the human to confirm, since ordering, placement, and tidying are the agent's and the human has seen only the sentences.
Write drafts where the user says, otherwise outside the source tree, since an intent file in the tree is authoritative the moment it exists; say where.
When code, tests, contract, and intent disagree, report the divergence; the human decides which is wrong.

## Admission

A statement belongs when all three hold:

> Would the software be wrong if tests passed but this were violated?
> Does it rule out a plausible change a human deliberately rejected, rather than restating the contract without naming the rejected alternative?
> Could a reviewer cite it to reject a change?

If it constrains how a contributor performs the work rather than the resulting software, it is `AGENTS.md`.
"Adding a variant requires an explicit decision" is process; the intent is "a variant without an explicit mapping is rejected, never given a fallback".
A test can enforce a property without recording why it exists; the intent beside it says what the test protects, so that an agent facing a red check escalates instead of deleting the test.
Default to no file: create one only when at least one commitment passes.

## Placement

`INTENT.md` at the narrowest directory covering all governed code; `<name>.intent.md` beside one file when a single module carries a policy its component does not.
Applicable files are the `INTENT*.md` in the directory and its ancestors plus the `<name>.intent*.md` beside the module itself, cumulative, and a specific file never overrides a broader one, so a narrowing is a contradiction a review catches rather than a silent exception; nothing follows a link, since a link breaks on rename and confers no applicability.
A design shared by several components is a part file, `INTENT.<feature>.md` or `<module>.intent.<feature>.md`, present in each governed directory; a symlink to one source keeps the parts identical, and where a symlink is not possible the copies are one text and a difference between them is a defect.
A part is read from several directories, so it names repository paths as code (`docs/agent-identity/CONTRACT.agent-identity.md`) rather than relative links.
A file per source file is the exception, never the default, and never mechanical.

## Anchors

A section that tests, code comments, or other documents cite or will cite carries an anchor on its own line just before its heading, and citations name the anchor rather than the heading text or number, because rewording a heading or inserting a section changes those and breaks every citation.
The anchor is `<a id="<section>"></a>`, a short kebab-case name of the section (`<a id="channel"></a>`) that no other section of the same file uses; the file is its namespace, because a citation names the file with the anchor, so only a repeat within the file makes it ambiguous.
An anchor keeps its id once published, even when its heading is reworded or its section renumbered; removing one breaks every citation of it, so the change that removes it updates them.
A file whose rules a test may cite has at least one heading, with an anchor on each cited section, rather than one flat list, because a flat list leaves a citation nothing narrower than the whole file.

## Shape

No mandatory sections, no title (the directory names the component), and no length budget; one commitment may be the whole file, and intent is read on demand by directory, like a doc comment, not loaded on every task.
A flat list for a handful that nothing cites; topical headings only when they aid retrieval or carry an anchor; `Overview` or `Purpose` only for a non-obvious role; `Non-goals` only for plausible, deliberately excluded behaviour.
A reason accompanies a commitment when it explains why a tempting alternative stays excluded; related commitments may share one, and reasons live in the same file as the commitments they support, so that two documents cannot drift apart.
One meaning per verb, because that is what lets a reader tell a requirement from advice: the declarative present or an imperative states a requirement, `should` a recommendation, `may` a permission, `can` a capability; never addressed to agents, since that constrains the contributor, not the software; sentences follow `prose-writer`.
Everything written normatively is to be realised by the time the branch merges; behaviour that is not yet to be built says so in the sentence ("TPM keys are wanted later"), so an implementer knows not to build it and a reviewer knows not to expect it.
Distinguish a present constraint that may change ("remains a CI-only check for now"), a deferral ("wanted later"), and a non-goal ("the intention is not to"); the first two are read differently by an audit, and the third tells a reader not to expect change.
State precision in the wording (`exactly these`, `at least`, `may vary provided`, `is permitted to`) rather than labelling the file exact or broad.
Give a conditional commitment a trigger or state and one response ("when a token is deleted, …", "while a rotation is in progress, …", "if the response is lost, then …"); one condition and one outcome per sentence is what a reviewer can check and an audit can split.
Scope every `never`, `only`, `always` to the boundary it protects and name the configurations a conditional commitment applies to, because a sibling, feature flag, or build across that boundary falsifies the unscoped form.
A scope sentence earns its place only when placement leaves a likely misapplication, stated once, next to the commitment it qualifies.
When the component introduces its own terms, a glossary entry per term opens with one or two sentences saying what it *is*, continues with its commitments, and ends with an example where the boundary is unclear and an *Avoid:* line where a competing word exists; a heading per term when entries carry commitments or will be cited, a flat `**Term**:` list otherwise; commitments that outgrow an entry move to their own section with a pointer from the entry.
A term belongs when this component gives it a meaning a reader could not have guessed, common word or not; a general programming concept used in its usual sense does not.
Specifics the human wants pinned (an exact set of variants, a dependency allowlist, a permitted tool, a pipeline order, a CLI's selector semantics) are intent even though they look like implementation; put them under the glossary entry of the concept they refine, or in a `<module>.intent.md` beside the one module they govern, and word them as the set or the permission.
The file still does not describe the implementation: incidental detail is the first thing a reviewer questions.
When a file grows, cut first what fails Admission.
A commitment that will be cited in reviews may carry a short name ("the single-classification rule") so it is referred to, not restated.

`references/pitfalls.md` lists how drafts have failed review and what the maintainer said; read it before handing a draft back or when reviewing.
`references/examples.md` points at the file the maintainer wrote by hand and says what makes it work; read it when unsure what a finished file looks like.
`references/handoff.md` is the handoff note and provenance table to write when handing drafts back.
