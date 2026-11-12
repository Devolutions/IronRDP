---
name: skill-discipline
description: Content standard for writing and reviewing agent skills (SKILL.md and references) - every sentence earns its place, one job per skill, rationale with each rule, no ceremony. Use when drafting, revising, or reviewing a skill's text.
---

A skill is loaded whole into an agent's context whenever its description matches, so each sentence costs every task that triggers it.
This skill is the test each sentence must pass; structure checks and evals belong to other tools.

## The test

A sentence belongs when the agent could not get it elsewhere cheaply: not from the tree, a manifest, `--help`, another skill it already loads, or its own competence.
"Integration tests exercise the CLI binary, not the library" passes; a command any agent knows does not.
Put positively: there is no way the agent learns this unless the skill says it.

## One job

A skill does one thing; the description names that thing and the moments it applies, in two sentences.
Distinct workflows that trigger separately are separate skills, and a skill may load the skills it builds on; a concept defined there is pointed to, not re-explained, while a lens or procedure is this skill's own even when its subject is defined elsewhere.
One sentence of premise from a dependency is allowed when it names the dependency and says what this skill does with it ("verdicts apply the test in `agents-md`"); a second sentence re-explaining it is not, and a lens may restate an authoring rule when the two skills are used independently, even if one names the other.
A `references/` file holds only material needed conditionally, named from the body with the condition that calls for it.

## Reasons where they pay

A rule the agent cannot see the point of is the first it drops under pressure, so a rule that is surprising, costly to get wrong, or tempting to "fix" carries its because in the same sentence or the next; a plain convention or a procedure step may stand without one, and a stated consequence serves a convention as a reason.
Prefer the reason to the imperative: "`should` is a recommendation, because one meaning per verb is what lets an audit tell a requirement from advice" over "always use `must` for requirements".
Write the mechanism, not the incident: "a second copy may drift", not "a second copy drifted and lost a code"; no stages, sessions, or decision references, since the reader is a future agent, not a participant.

## No ceremony

No "Overview" that restates the description, no closing summary, no mandatory headings; an example earns its place when it shows a rewrite (bad form to good) the rule alone leaves ambiguous, and one that only instances the rule is ceremony, while an inline form token or an output template is the rule's own wording.

Omit an opening title that only repeats the skill name or frontmatter description, because that context is already available.
Keep headings that organize the instructions or add context.

## Words

Plain language at the sentence level; one sentence per line; the same word for the same thing throughout.
Terms the skill coins are defined where first used, with the words it avoids when a competing term exists.
Address the agent as the reader doing the task; never describe what agents are like.

## Reviewing a skill

Quote each sentence that fails the test and name which part: discoverable, re-explains a concept another skill defines, surprising rule without its reason, ceremony, two jobs, incident for mechanism, describes what agents are like instead of stating the rule (a mechanism about the artifact's readers is not this).
Ask "why" of any confident rule before accepting it; procedure steps and conventions are exempt.
Propose the shorter sentence, the deletion, or the sentence with its reason; do not edit, and do not compress past meaning.
Report what is strong as written, so it is not touched.
