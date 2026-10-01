"use strict";

const { readCheckRuns } = require("./check-runs");
const { readLatestExactHeadCiRun } = require("./ci-state");
const { canonicalRuns, ownerIsActive, parseLeaseMarker } = require("./automation-lease");
const { parseCheckState } = require("./validate-classifier");
const { trustedReviewOutcome } = require("./review-outcome");

const ACTOR_LABELS = ["needs-review", "needs-author-action"];

function names(labels) {
  return new Set((labels || []).map((label) => typeof label === "string" ? label : label?.name).filter(Boolean));
}

function canonical(runs, kind, headSha) {
  return canonicalRuns(runs || [], { kind, headSha });
}

async function activeLease(github, owner, repo, run, headSha) {
  const lease = parseLeaseMarker(run?.output?.summary);
  return run?.status === "in_progress" && lease?.headSha === headSha &&
    await ownerIsActive(github, { ...lease, owner, repo });
}

function lifecycleActor(snapshot) {
  if (snapshot.state !== "open" || snapshot.draft || snapshot.activeLease) return [];
  const ci = snapshot.ci;
  if (!ci || ci.status !== "completed") return [];
  if (snapshot.canonicalAmbiguous) return [];
  if (ci.conclusion === "failure") return ["needs-author-action"];
  if (ci.conclusion !== "success") return [];
  if (snapshot.untrustedReviewSuccess) return null;
  if (snapshot.labels.has("automation-failed")) return [];
  if (snapshot.reviewOutcome === "findings") return ["needs-author-action"];
  if (snapshot.reviewOutcome === "no-findings") return ["needs-review"];
  if (snapshot.classificationValid &&
      (snapshot.labels.has("triage/legitimacy") || snapshot.labels.has("ai-reviewed/2"))) {
    return ["needs-review"];
  }
  return [];
}

async function readLifecycleSnapshot({ github, owner, repo, prNumber, observedRun = null, ciRetry = {} }) {
  const { data: pull } = await github.rest.pulls.get({ owner, repo, pull_number: prNumber });
  const labels = names((await github.rest.issues.get({ owner, repo, issue_number: prNumber })).data.labels);
  const headSha = pull.head?.sha;
  if (!/^[0-9a-f]{40}$/.test(headSha || "") || pull.state !== "open" || pull.draft) {
    return { state: pull.state, draft: pull.draft === true, labels, headSha, activeLease: false };
  }
  const [classificationRuns, reviewRuns, ci] = await Promise.all([
    readCheckRuns({ github, owner, repo, ref: headSha, checkName: "AI classification" }),
    readCheckRuns({ github, owner, repo, ref: headSha, checkName: "AI automated review" }),
    readLatestExactHeadCiRun({ github, owner, repo, expectedSha: headSha, observedRun, ...ciRetry }),
  ]);
  const classifications = canonical(classificationRuns, "classification", headSha);
  const reviews = canonical(reviewRuns, "review", headSha);
  const latestClassification = classifications?.[0] ?? null;
  const latestReview = reviews?.[0] ?? null;
  const [classificationLease, reviewLease] = await Promise.all([
    activeLease(github, owner, repo, latestClassification, headSha),
    activeLease(github, owner, repo, latestReview, headSha),
  ]);
  const successfulReviews = reviews?.filter((run) => run.conclusion === "success") ?? [];
  const newestSuccessfulReview = successfulReviews[0] ?? null;
  const reviewOutcome = trustedReviewOutcome(newestSuccessfulReview, headSha);
  return {
    state: pull.state,
    draft: pull.draft === true,
    labels,
    headSha,
    ci,
    activeLease: classificationLease || reviewLease,
    canonicalAmbiguous: classifications === null || reviews === null,
    classificationValid: latestClassification?.conclusion === "success" &&
      ["Classification complete", "Automation stopped"].includes(latestClassification.output?.title) &&
      parseCheckState(latestClassification.output?.summary) !== null,
    reviewOutcome,
    untrustedReviewSuccess: newestSuccessfulReview !== null && reviewOutcome === null,
  };
}

async function reconcileLifecycle({ github, owner, repo, prNumber, observedRun = null, ciRetry = {} }) {
  let changed = false;
  let actor = [];
  // One bounded verification pass converges a transition that races a write without creating a loop.
  for (let pass = 0; pass < 2; pass += 1) {
    const snapshot = await readLifecycleSnapshot({
      github, owner, repo, prNumber, observedRun, ciRetry,
    });
    const desired = lifecycleActor(snapshot);
    if (desired === null) {
      return { ok: true, changed, reason: "review outcome receipt unavailable" };
    }
    actor = desired;
    const remove = ACTOR_LABELS.filter((label) => snapshot.labels.has(label) && !desired.includes(label));
    const add = desired.filter((label) => !snapshot.labels.has(label));
    if (remove.length === 0 && add.length === 0) break;

    for (const label of remove) {
      const current = await readLifecycleSnapshot({
        github, owner, repo, prNumber, observedRun, ciRetry,
      });
      const currentDesired = lifecycleActor(current);
      if (currentDesired === null) return { ok: true, changed, reason: "review outcome receipt unavailable" };
      if (!current.labels.has(label) || currentDesired.includes(label)) continue;
      try {
        await github.rest.issues.removeLabel({ owner, repo, issue_number: prNumber, name: label });
        changed = true;
      } catch (error) {
        if (error?.status !== 404) throw error;
      }
    }

    const current = await readLifecycleSnapshot({
      github, owner, repo, prNumber, observedRun, ciRetry,
    });
    const currentDesired = lifecycleActor(current);
    if (currentDesired === null) return { ok: true, changed, reason: "review outcome receipt unavailable" };
    const additions = currentDesired.filter((label) => !current.labels.has(label));
    if (additions.length > 0) {
      await github.rest.issues.addLabels({ owner, repo, issue_number: prNumber, labels: additions });
      changed = true;
    }
    actor = currentDesired;
  }
  return { ok: true, changed, actor };
}

module.exports = {
  ACTOR_LABELS, canonical, lifecycleActor, readLifecycleSnapshot, reconcileLifecycle,
};
