---
name: review-feedback-loop
description: Work through findings on an open pull request from Copilot, a review pipeline, or human reviewers - fix what is substantive, answer a wrong finding with the governing intent or contract, one review round per change, until a round returns nothing you accept. Use when asked to address review comments or bring a pull request with outstanding findings to approval.
---

A finding is a claim about the code: verify it before acting on it, and loop until a round returns no finding you accept, since a reviewer that saw the fixes is the only evidence they held.

## Triage

- Correctness, security, intent, contract, or style the repository's rules decide: fix.
- Wrong, or in conflict with an intent file or `CONTRACT.md`: reply with the file and clause, change nothing.
- A decision no intent, contract, or brief states: escalate, because a fix is the least reviewed place for a decision to land.
- A change to an intent file, `CONTRACT.md`, or blind suite this pull request does not own: escalate; it belongs to the pull request that owns the text.

A logic fix takes the path the original code took - worker, reviewer, lead verification - when a `lead` loop exists; a trivial fix (typo, wording, comment) is applied directly.

## Reply and the next round

Reply to each thread in one sentence with the fixing commit or the answering clause, with the repository's comment footer when one is defined.
Resolve the automated threads you fixed; leave a human's thread for the human unless the repository's convention says otherwise.
One review round per substantive change and none after a comment-only one, since a round costs minutes and returns findings proportional to the diff it sees.
Watch for the round's result - the reviewer's new review, or its check run where the pipeline posts one - and triage it when it lands, because the loop is open until someone reads the verdict.
Where a pipeline reviews every pushed head on its own, the push is the request: batch a round's fixes into one push, because each head spends one of a bounded number of reviews, and answer findings that need no code change with the pipeline's hand-off command rather than a push.

## Stop

- A round returns no finding, or only findings you answered without a change: record the verdict and stop; the human judges the answers, since the same diff re-reviewed returns the same claims.
- A fix that reintroduces an earlier finding: escalate with both; alternating fixes converge on nothing.
- Only a human's approval remains: report the head and stop, since a human's turnaround is unbounded and watching for it burns a session for nothing.

Fetch before pushing; when the branch has moved, rebase onto its new head so the human's commits stay in it, and force only with lease pinned to that head, because a lease guards against pushes after your fetch, not before it.
Record the reviewer's final verdict verbatim with the head SHA it applies to.
