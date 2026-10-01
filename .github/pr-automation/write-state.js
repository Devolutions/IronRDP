"use strict";

const { encodeCheckState } = require("./validate-classifier");
const { REVIEWERS, provenancePrefix } = require("./validate-final-review");
const { readCheckRuns } = require("./check-runs");
const {
  MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS, escapeMarkdown, inlineReviewCommentBody,
  reducedCoverageText, reviewBody,
} = require("./review-render");
const { reviewPolicyEligible } = require("./routing");
const { ownsActiveLease, parseLeaseMarker } = require("./automation-lease");
const { assertCurrentHead } = require("./current-head");
const { matchesGeneration, readLatestExactHeadCiRun } = require("./ci-state");
const { ACTOR_LABELS, reviewCount } = require("./resolve-state");

class StalePolicyError extends Error {
  constructor() { super("pull request review policy changed"); this.name = "StalePolicyError"; }
}

async function issueLabels(github, owner, repo, prNumber) {
  const { data } = await github.rest.issues.get({ owner, repo, issue_number: prNumber });
  return new Set(data.labels.map((label) => typeof label === "string" ? label : label.name).filter(Boolean));
}

async function comments(github, owner, repo, prNumber) {
  const result = [];
  for await (const response of github.paginate.iterator(github.rest.issues.listComments, {
    owner, repo, issue_number: prNumber, per_page: 100,
  })) result.push(...response.data);
  return result;
}

function markerBody(comment, owner, repo) {
  if (comment.kind === "overlap") {
    return `${comment.marker}\n\nThis pull request may overlap with ${escapeMarkdown(comment.url)}.\n\n${escapeMarkdown(comment.rationale)}\n\nThis notice is advisory only. Automated review continues as usual, and how these pull requests relate is for maintainers and authors to decide.\n\n> [!NOTE]\n> LLM-assisted content (no human feedback).`;
  }
  if (comment.kind === "legitimacy") {
    return `${comment.marker}\n\nAutomated review stopped because commit \`${escapeMarkdown(comment.sha)}\` has strong indicators requiring legitimacy triage.\n\n${escapeMarkdown(comment.reason)}\n\nThis comment remains as an audit record if later classifications differ. When CI succeeds for this exact head, automation will hand it to a human reviewer.`;
  }
  if (comment.kind === "global-quota") {
    return `${comment.marker}\n\nAutomated classification and review capacity for fork pull requests has reached its daily UTC limit.\n\nSee the [automation policy](https://github.com/${owner}/${repo}/blob/master/.github/PR_AUTOMATION.md). Automation remains blocked until capacity is available and it is retried.`;
  }
  if (comment.kind === "evidence-limit") {
    const guidance = comment.limitMiB === 1
      ? "A maintainer can add `ai-review/allow-oversized` to retry with the runtime maximum of 4 MiB. Otherwise, split the change into focused pull requests or reduce generated content."
      : "The 4 MiB limit is the model runtime maximum. Please split the change into focused pull requests or reduce generated content.";
    return `${comment.marker}\n\nAutomated model analysis stopped because this pull request's diff exceeds the ${comment.limitMiB} MiB evidence limit. No model was invoked with partial evidence.\n\n${guidance} Automation remains blocked until retry or repair.`;
  }
  throw new Error("unsupported issue comment");
}

async function upsertMarkedComment(
  github, owner, repo, prNumber, expectedSha, botLogin, comment, canMutate, expectedBaseSha,
) {
  if (!botLogin || typeof botLogin !== "string") throw new Error("botLogin is required for comment ownership");
  const body = markerBody(comment, owner, repo);
  const existing = (await comments(github, owner, repo, prNumber)).find((item) =>
    item.user?.login === botLogin && typeof item.body === "string" && item.body.includes(comment.marker));
  if (existing?.body === body) return false;
  await issueLabels(github, owner, repo, prNumber);
  await assertCurrentHead({
    github, owner, repo, pullNumber: prNumber, expectedHeadSha: expectedSha, expectedBaseSha,
  });
  if (canMutate && !await canMutate()) return false;
  if (existing) {
    await github.rest.issues.updateComment({ owner, repo, comment_id: existing.id, body });
  } else {
    await github.rest.issues.createComment({ owner, repo, issue_number: prNumber, body });
  }
  return true;
}

async function deleteMarkedComment(
  github, owner, repo, prNumber, expectedSha, botLogin, marker, canMutate, expectedBaseSha,
) {
  if (!botLogin || typeof botLogin !== "string") throw new Error("botLogin is required for comment ownership");
  const existing = (await comments(github, owner, repo, prNumber)).find((item) =>
    item.user?.login === botLogin && typeof item.body === "string" && item.body.includes(marker));
  if (!existing) return false;
  await assertCurrentHead({
    github, owner, repo, pullNumber: prNumber, expectedHeadSha: expectedSha, expectedBaseSha,
  });
  if (canMutate && !await canMutate()) return false;
  await github.rest.issues.deleteComment({ owner, repo, comment_id: existing.id });
  return true;
}

async function reviews(github, owner, repo, prNumber) {
  const result = [];
  for await (const response of github.paginate.iterator(github.rest.pulls.listReviews, {
    owner, repo, pull_number: prNumber, per_page: 100,
  })) result.push(...response.data);
  return result;
}

function assertReviewPolicy(labels, state) {
  const currentReviewCount = reviewCount(labels);
  if (state.handoff === "terminal") {
    if (!labels.has("ai-reviewed/2")) throw new StalePolicyError();
    return;
  }
  if (state.handoff === "legitimacy") {
    if (!labels.has("triage/legitimacy")) throw new StalePolicyError();
    return;
  }
  if (currentReviewCount !== state.expectedReviewCount ||
      (!state.forced && !reviewPolicyEligible({
        labels: [...labels],
      }))) {
    throw new StalePolicyError();
  }
}

function requiresFreshCi(state) {
  return state.mode === "review" && state.forced !== true && state.failed !== true && state.blocked !== true;
}

// A listing that has not caught up to the authorized generation is retried briefly rather than
// treated as stale. A newer generation is returned without retry and still fails the check.
async function assertFreshCi(github, owner, repo, state, ciRetry = {}) {
  if (!requiresFreshCi(state)) return;
  const latest = await readLatestExactHeadCiRun({
    github, owner, repo, expectedSha: state.expectedSha,
    expectedGeneration: { id: state.ciRunId, attempt: state.ciRunAttempt }, ...ciRetry,
  });
  if (latest?.conclusion !== "success" ||
      !matchesGeneration(latest, state.ciRunId, state.ciRunAttempt)) {
    throw new StalePolicyError();
  }
}

async function publishReview(
  github, owner, repo, prNumber, state, botLogin, comment, canMutate, ciRetry,
) {
  if (!botLogin || typeof botLogin !== "string") throw new Error("botLogin is required for review ownership");
  const review = comment.review;
  const reducedCoverage = comment.reducedCoverage ?? [];
  if (!Array.isArray(reducedCoverage) || new Set(reducedCoverage).size !== reducedCoverage.length ||
      reducedCoverage.some((reviewer) => !REVIEWERS.includes(reviewer))) {
    throw new Error("invalid review coverage");
  }
  const inline = review.findings.filter((finding) => finding.start_line !== null).map((finding) => {
    const comment = {
      path: finding.path, line: finding.end_line, side: "RIGHT",
      body: inlineReviewCommentBody(finding, provenancePrefix),
    };
    if (finding.start_line !== finding.end_line) {
      comment.start_line = finding.start_line;
      comment.start_side = "RIGHT";
    }
    return comment;
  });
  const body = reviewBody(comment.marker, review, reducedCoverage, provenancePrefix);
  if (body.length > MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS ||
      inline.some((entry) => entry.body.length > MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS)) {
    throw new Error("review publication exceeds GitHub body limit");
  }
  if ((await reviews(github, owner, repo, prNumber)).some((published) =>
    published.user?.login === botLogin &&
    typeof published.body === "string" &&
    published.body.includes(comment.marker))) return false;
  await assertFreshCi(github, owner, repo, state, ciRetry);
  await assertCurrentHead({
    github, owner, repo, pullNumber: prNumber, expectedHeadSha: state.expectedSha,
  });
  assertReviewPolicy(await issueLabels(github, owner, repo, prNumber), state);
  if (canMutate && !await canMutate()) return false;
  await github.rest.pulls.createReview({
    owner, repo, pull_number: prNumber, commit_id: state.expectedSha, event: "COMMENT",
    body, comments: inline,
  });
  return true;
}

async function findChecks(github, owner, repo, expectedSha, check) {
  const runs = await readCheckRuns({
    github, owner, repo, ref: expectedSha, checkName: check.name,
  });
  const matching = runs.filter((run) => run.external_id === check.externalId);
  if (matching.some((run) => run.app?.slug !== "github-actions")) {
    throw new Error("canonical check is ambiguous");
  }
  return matching;
}

function newestRun(runs) {
  return runs.reduce((found, run) => !found || run.id > found.id ? run : found, null);
}

async function ensureCanonicalCheck({
  github, owner, repo, prNumber, expectedSha, expectedBaseSha, check, state,
  title, summary, canMutate, beforeWrite,
}) {
  const conclusion = check.conclusion ?? "success";
  const runs = await findChecks(github, owner, repo, expectedSha, check);
  const successful = runs.some((run) => run.conclusion === "success");
  if (successful && conclusion !== "success") return false;
  if (state.forced === true && conclusion !== "success" &&
      runs.some((run) => run.status === "in_progress" && parseLeaseMarker(run.output?.summary))) {
    return false;
  }
  const existing = state.lease ? { id: state.lease.checkRunId }
    : state.forced === true && conclusion === "success" ? null
    : newestRun(runs);
  if ((successful && !state.lease && state.forced !== true) ||
      (existing?.conclusion === conclusion && existing.output?.title === title &&
      existing.output?.summary === summary)) return false;
  if (beforeWrite) await beforeWrite();
  await assertCurrentHead({
    github, owner, repo, pullNumber: prNumber, expectedHeadSha: expectedSha, expectedBaseSha,
  });
  if (canMutate && !await canMutate()) return false;
  const payload = {
    owner, repo, name: check.name, head_sha: expectedSha, external_id: check.externalId,
    status: "completed", conclusion,
    output: { title, summary },
  };
  if (existing) {
    const { owner, repo, status, conclusion, output } = payload;
    await github.rest.checks.update({
      owner, repo, check_run_id: existing.id, status, conclusion, output,
    });
  } else {
    await github.rest.checks.create(payload);
  }
  return true;
}

async function ensureClassificationCheck(
  github, owner, repo, prNumber, expectedSha, expectedBaseSha, check, state, canMutate,
) {
  return ensureCanonicalCheck({
    github, owner, repo, prNumber, expectedSha, expectedBaseSha, check, state, canMutate,
    title: check.title,
    summary: `${check.summary}\n\n${encodeCheckState(check.machineState)}`,
  });
}

async function ensureReviewCheck(
  github, owner, repo, prNumber, expectedSha, check, state, canMutate, ciRetry,
) {
  const title = check.title ?? "Automated review complete";
  const summary = check.summary ?? "Validated automated review is bound to this commit.";
  return ensureCanonicalCheck({
    github, owner, repo, prNumber, expectedSha, check, state, title, summary, canMutate,
    beforeWrite: state.failed === true || state.blocked === true ? null : async () => {
      await assertFreshCi(github, owner, repo, state, ciRetry);
      assertReviewPolicy(await issueLabels(github, owner, repo, prNumber), state);
    },
  });
}

async function dispatchClassificationComplete(
  github, owner, repo, prNumber, expectedSha, expectedBaseSha,
) {
  await assertCurrentHead({
    github, owner, repo, pullNumber: prNumber, expectedHeadSha: expectedSha, expectedBaseSha,
  });
  await github.rest.repos.createDispatchEvent({
    owner, repo, event_type: "pr-automation-classified",
    client_payload: { pr_number: prNumber, head_sha: expectedSha },
  });
}

// Computes the whole label delta from a single read, so a state with two label sets plus additions
// and removals costs one issue read instead of one per candidate label.
async function applyLabels(github, owner, repo, prNumber, state, currentLabels, canMutate) {
  const current = currentLabels ?? await issueLabels(github, owner, repo, prNumber);
  const add = new Set();
  const remove = new Set();
  for (const { owned, desired } of state.labelSets || []) {
    const wanted = new Set(desired || []);
    for (const label of new Set(owned || [])) (wanted.has(label) ? add : remove).add(label);
  }
  for (const label of state.addLabels || []) add.add(label);
  for (const label of state.removeLabels || []) { add.delete(label); remove.add(label); }
  const additions = [...add].filter((label) => !current.has(label));
  const removals = [...remove].filter((label) => current.has(label));
  if (additions.length === 0 && removals.length === 0) return false;
  await assertCurrentHead({
    github, owner, repo, pullNumber: prNumber, expectedHeadSha: state.expectedSha,
    expectedBaseSha: state.expectedBaseSha,
  });
  // The actor labels are mutually exclusive. Remove a stale actor before adding its replacement so
  // a failed add leaves no actor selected rather than two conflicting actors. Other labels retain
  // the established add-before-remove ordering.
  const actorAdditions = new Set(additions.filter((label) => ACTOR_LABELS.includes(label)));
  const actorRemovals = new Set(removals.filter((label) => ACTOR_LABELS.includes(label)));
  if (actorAdditions.size > 0) {
    for (const label of ACTOR_LABELS) {
      if (current.has(label) && !actorAdditions.has(label)) actorRemovals.add(label);
    }
  }
  const removeLabel = async (label) => {
    if (canMutate && !await canMutate()) return false;
    try {
      await github.rest.issues.removeLabel({ owner, repo, issue_number: prNumber, name: label });
    } catch (error) {
      if (error?.status !== 404) throw error;
    }
  };
  for (const label of actorRemovals) await removeLabel(label);
  if (additions.length > 0) {
    if (canMutate && !await canMutate()) return false;
    await github.rest.issues.addLabels({ owner, repo, issue_number: prNumber, labels: additions });
  }
  for (const label of removals.filter((label) => !ACTOR_LABELS.includes(label))) {
    await removeLabel(label);
  }
  return true;
}

async function completeNeutralCheck(github, owner, repo, checkRunId, marker, title) {
  await github.rest.checks.update({
    owner, repo, check_run_id: checkRunId, status: "completed", conclusion: "neutral",
    output: { title, summary: marker },
  });
}

async function completeLeaseNeutral(github, owner, repo, state, canMutate, title) {
  if (!state.lease || !await canMutate()) return false;
  await completeNeutralCheck(
    github, owner, repo, state.lease.checkRunId, state.lease.marker, title);
  return true;
}

async function writeState({
  github, owner, repo, prNumber, state, botLogin, reviewRequested = false, ciRetry = {},
}) {
  if (!state?.ok || !["classification", "review"].includes(state.mode) ||
      typeof state.expectedSha !== "string" || !Number.isSafeInteger(prNumber) || prNumber <= 0) {
    throw new Error("invalid normalized state");
  }
  const canMutate = async () => !state.lease || state.forced === true || await ownsActiveLease({ github, owner, repo, lease: state.lease });
  if (state.superseded === true) {
    await completeLeaseNeutral(
      github, owner, repo, state, canMutate, "Automation superseded");
    return { ok: true, superseded: true };
  }
  await assertCurrentHead({
    github, owner, repo, pullNumber: prNumber, expectedHeadSha: state.expectedSha,
    expectedBaseSha: state.expectedBaseSha,
  });
  const check = state.check;
  let existingCheck = null;
  let successfulCheck = null;
  if (check) {
    const checks = await findChecks(github, owner, repo, state.expectedSha, check);
    existingCheck = checks.reduce((found, run) => !found || run.id > found.id ? run : found, null);
    successfulCheck = checks.find((run) => run.conclusion === "success") ?? null;
    const nonSuccess = state.failed === true || state.blocked === true ||
      (check.conclusion ?? "success") !== "success";
    if (successfulCheck && nonSuccess) {
      if (state.lease) {
        await completeLeaseNeutral(
          github, owner, repo, state, canMutate, "Automation superseded");
      }
      return { ok: true, superseded: true };
    }
    if (state.forced === true && nonSuccess &&
        checks.some((run) => run.status === "in_progress" &&
          parseLeaseMarker(run.output?.summary))) {
      return { ok: true, superseded: true };
    }
    if (state.forced === true && !nonSuccess) {
      const automatic = checks.filter((run) => {
        const lease = parseLeaseMarker(run.output?.summary);
        return run.status === "in_progress" && lease?.kind === state.mode &&
          lease.headSha === state.expectedSha;
      });
      for (const run of automatic) {
        await completeNeutralCheck(
          github, owner, repo, run.id, run.output.summary, "Automation superseded");
      }
    }
  }
  if (!await canMutate()) return { ok: true, superseded: true };
  if (state.mode === "review") {
    const comments = state.comments || [];
    for (const comment of comments.filter((comment) => comment.kind === "review")) {
      if (!await canMutate()) return { ok: true, superseded: true };
      await publishReview(
        github, owner, repo, prNumber, state, botLogin, comment, canMutate, ciRetry);
    }
    const latestLabels = state.failed === true || state.blocked === true
      ? undefined
      : await issueLabels(github, owner, repo, prNumber);
    if (latestLabels) {
      await assertFreshCi(github, owner, repo, state, ciRetry);
      assertReviewPolicy(latestLabels, state);
    }
    if (!await canMutate()) return { ok: true, superseded: true };
    await applyLabels(github, owner, repo, prNumber, state, latestLabels, canMutate);
    for (const comment of comments.filter((comment) => comment.kind !== "review")) {
      if (!await canMutate()) return { ok: true, superseded: true };
      await upsertMarkedComment(
        github, owner, repo, prNumber, state.expectedSha, botLogin, comment, canMutate);
    }
    for (const marker of new Set(state.removeCommentMarkers || [])) {
      if (!await canMutate()) return { ok: true, superseded: true };
      await deleteMarkedComment(
        github, owner, repo, prNumber, state.expectedSha, botLogin, marker, canMutate);
    }
    if (state.check) {
      if (!await canMutate()) return { ok: true, superseded: true };
      await ensureReviewCheck(
        github, owner, repo, prNumber, state.expectedSha, state.check, state, canMutate, ciRetry);
    } else if (state.lease && !await completeLeaseNeutral(
      github, owner, repo, state, canMutate, "Automation blocked",
    )) {
      return { ok: true, superseded: true };
    }
  } else {
    if (!await canMutate()) return { ok: true, superseded: true };
    await applyLabels(github, owner, repo, prNumber, state, undefined, canMutate);
    for (const comment of state.comments || []) await upsertMarkedComment(
      github, owner, repo, prNumber, state.expectedSha, botLogin, comment, canMutate,
      state.expectedBaseSha);
    for (const comment of state.auditComments || []) await upsertMarkedComment(
      github, owner, repo, prNumber, state.expectedSha, botLogin, comment, canMutate,
      state.expectedBaseSha);
    for (const marker of new Set(state.removeCommentMarkers || [])) {
      await deleteMarkedComment(
        github, owner, repo, prNumber, state.expectedSha, botLogin, marker, canMutate,
        state.expectedBaseSha);
    }
    if (state.check) {
      if (!await canMutate()) return { ok: true, superseded: true };
      const changed = await ensureClassificationCheck(
        github, owner, repo, prNumber, state.expectedSha, state.expectedBaseSha,
        state.check, state, canMutate);
      const shouldDispatch = ["Classification complete", "Automation stopped"].includes(
        state.check.title,
      ) && state.dispatchReview !== false && (state.lease
        ? changed
        : (reviewRequested || existingCheck === null) &&
          successfulCheck === null &&
          (changed || reviewRequested || state.check.title === "Automation stopped"));
      if (shouldDispatch) {
        await dispatchClassificationComplete(
          github, owner, repo, prNumber, state.expectedSha, state.expectedBaseSha,
        );
      }
    }
  }
  return { ok: true };
}

module.exports = {
  StalePolicyError, applyLabels, deleteMarkedComment, dispatchClassificationComplete,
  escapeMarkdown, markerBody, upsertMarkedComment, writeState,
  reducedCoverageText,
};
