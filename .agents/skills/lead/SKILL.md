---
name: lead
description: Deliver one pull request from a brief with sub-agents - workers, a fresh-context reviewer on a frozen commit, your own verification, and a report the reader can verify. Use when a change needs its own session and more than one pair of hands, assigned by an orchestrator or by the user.
---

# Lead

You own one pull request from brief to merge-ready and are its last check; a second pull request gets a second session, so each report, review loop, and branch has one owner.

## Start

Read the brief, then every applicable intent file, `CONTRACT.md`, and `AGENTS.md` for the paths in scope.
Models: those the brief names, else ask the user before the first sub-agent, recommending from the options you can see - strong for reviewers and normative work, and a reviewer family different from the writer's when available, since one family shares its blind spots.
Write the plan as an artifact outside the tree, never committed, with the acceptance criteria restated as checks you will run.

## Decisions

A decision the brief, intent files, and contract do not settle is not yours: escalate with options, how each could go wrong, and a recommendation, and let no worker encode it in a test or code meanwhile, since whatever lands first becomes the text the rest is bent to.
Within autonomy - naming inside repository conventions, test structure, which published vector, a classification a stated rule decides, non-semantic documentation - decide, record in the plan, flag in the report.
Two approaches within autonomy with trade-offs paper cannot settle are a `variants` tournament, run as sub-agents before committing to either.
A change to an intent file, `CONTRACT.md`, or blind suite that this pull request does not own never rides it: escalate, and wait or stack on the pull request that carries it.

## Sub-agents

One task per sub-agent, briefed from an artifact per `references/templates.md`, pointing at paths rather than copying them so the sub-agent reads current text, and listing what it must not read as well as what it must, because isolation is only what the brief denies.
Workers write code and tests in your worktree or branch, each behaviour through `tdd` or against the blind suite's failing checks where one exists, and never push or open the pull request; testers follow `test-writer`, suite authors `blind-testsuite`.

## Review loop

Review a commit, not a working tree: the worker commits, hands over the range, and stops editing until the verdict, since a change during review means the reviewer judged something that no longer exists.
The reviewer gets the range, the acceptance criteria, the applicable intent, contract, and `AGENTS.md` paths, and an instruction to run `code-review`; never the worker's transcript or reasoning, because the writer's framing anchors a reviewer before it reads the evidence.
Blocking findings go back to the worker; non-blocking ones are fixed or the reason not to is recorded in the plan.
Set the round cap in the plan before the first round; at the cap, outstanding fixes go to a fresh reviewer rather than out unreviewed, because post-cap fixes are exactly where unreviewed defects land.
A round that reintroduces an earlier finding ends the loop: escalate with both findings.
After the last feature edit and before the last review round, a worker runs `dependency-pruner` when a manifest or lockfile changed, then another runs `test-pruner` in its editing mode on the diff, in that order because a replaced package changes the code the mutants measure; the final range the reviewer sees includes their commits, and the pruner reports travel with it, since `dependency-reviewer` reads the dependency report as the evidence that an addition is load-bearing.
Post both pruners' evidence once the pull request exists.

## Your verification

Verdicts are claims; verify before opening the pull request, whatever the loop returned: build, run the full suite, read the whole diff, and read line by line any code handling untrusted input, keys, authentication, or a `CONTRACT.md` surface.
Check each acceptance criterion against evidence you produced, byte-compare approved normative text copied into the tree against its approved artifact, and run `intent-drift-audit` on governed paths, since the realisation is now claimed complete.

## Pull request and report

Open the pull request per the repository's conventions, and put the acceptance evidence, deviations from the brief, and open questions in a comment, because a body that becomes the squash commit carries rationale, not process; run `review-feedback-loop` once findings arrive.
Report per `references/templates.md`: head SHA and CI run let the reader verify instead of trust, and every non-success job is named, because a summary count hides the one that matters.
Merging is the human's: report, mark the status waiting, and stop rather than polling.
