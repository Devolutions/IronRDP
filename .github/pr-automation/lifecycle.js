"use strict";

const { readCheckRuns } = require("./check-runs");
const { readLatestExactHeadCiRun } = require("./ci-state");
const { canonicalRuns, ownerIsActive, parseLeaseMarker } = require("./automation-lease");
const { parseCheckState } = require("./validate-classifier");
const { trustedReviewOutcome, trustedReviewReceipt, reviewMarkerPrefix } = require("./review-outcome");
const { ACTOR_LABELS, labelsOf } = require("./resolve-state");
const { AI_COUNTS, reviewCount } = require("./review-count");

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
  if (reviewCount(snapshot.labels) === undefined) return [];
  if (ci.conclusion !== "success") return [];
  if (snapshot.untrustedReviewSuccess) return null;
  if (snapshot.labels.has("automation-failed")) return [];
  if (snapshot.reviewOutcome === "findings") return ["needs-author-action"];
  if (snapshot.reviewOutcome === "no-findings") return ["needs-review"];
  if (snapshot.classificationValid && reviewCount(snapshot.labels) === AI_COUNTS.at(-1) &&
      !snapshot.priorReviewTrusted && !snapshot.labels.has("triage/legitimacy")) return null;
  if (snapshot.classificationValid &&
      (snapshot.labels.has("triage/legitimacy") ||
       (reviewCount(snapshot.labels) === AI_COUNTS.at(-1) && snapshot.priorReviewTrusted))) {
    return ["needs-review"];
  }
  return [];
}

async function readLifecycleSnapshot({ github, owner, repo, prNumber, observedRun = null, ciRetry = {} }) {
  const { data: pull } = await github.rest.pulls.get({ owner, repo, pull_number: prNumber });
  const labels = labelsOf((await github.rest.issues.get({ owner, repo, issue_number: prNumber })).data.labels);
  const headSha = pull.head?.sha;
  if (!/^[0-9a-f]{40}$/.test(headSha || "") || pull.state !== "open" || pull.draft) {
    return { state: pull.state, draft: pull.draft === true, labels, headSha, activeLease: false };
  }
  const [classificationRuns, reviewRuns, ci] = await Promise.all([
    readCheckRuns({ github, owner, repo, ref: headSha, checkName: "AI classification" }),
    readCheckRuns({ github, owner, repo, ref: headSha, checkName: "AI automated review" }),
    readLatestExactHeadCiRun({ github, owner, repo, expectedSha: headSha, observedRun, ...ciRetry }),
  ]);
  const classifications = canonicalRuns(classificationRuns || [], { kind: "classification", headSha });
  const reviews = canonicalRuns(reviewRuns || [], { kind: "review", headSha });
  const latestClassification = classifications?.[0] ?? null;
  const latestReview = reviews?.[0] ?? null;
  const [classificationLease, reviewLease] = await Promise.all([
    activeLease(github, owner, repo, latestClassification, headSha),
    activeLease(github, owner, repo, latestReview, headSha),
  ]);
  const successfulReviews = reviews?.filter((run) => run.conclusion === "success") ?? [];
  const newestSuccessfulReview = successfulReviews[0] ?? null;
  const reviewOutcome = trustedReviewOutcome(newestSuccessfulReview, headSha);
  const classificationValid = latestClassification?.conclusion === "success" &&
    ["Classification complete", "Automation stopped"].includes(latestClassification.output?.title) &&
    parseCheckState(latestClassification.output?.summary) !== null;
  let priorReviewTrusted = false;
  if (reviewCount(labels) === AI_COUNTS.at(-1) && !newestSuccessfulReview &&
      ci?.conclusion === "success" && classificationValid &&
      !classificationLease && !reviewLease) {
    // The latest published bot review identifies the head that spent the terminal count.
    // A label alone (or a legacy successful check without a receipt) is not evidence.
    const published = [];
    for await (const page of github.paginate.iterator(github.rest.pulls.listReviews, {
      owner, repo, pull_number: prNumber, per_page: 100,
    })) published.push(...page.data);
    const latest = published.filter((review) =>
      review.user?.login === "github-actions[bot]")
      .sort((left, right) => right.id - left.id)[0];
    if (latest && /^[0-9a-f]{40}$/.test(latest.commit_id || "") &&
        latest.body?.startsWith(reviewMarkerPrefix(latest.commit_id)) &&
        latest.commit_id !== headSha) {
      const priorRuns = canonicalRuns(await readCheckRuns({
        github, owner, repo, ref: latest.commit_id, checkName: "AI automated review",
      }) || [], { kind: "review", headSha: latest.commit_id });
      const prior = priorRuns?.[0];
      const receipt = trustedReviewReceipt(prior, latest.commit_id);
      priorReviewTrusted = receipt?.next_review_count === AI_COUNTS.at(-1) &&
        (latest.body === receipt.review_marker ||
         latest.body.startsWith(`${receipt.review_marker}\n`));
    }
  }
  return {
    state: pull.state,
    draft: pull.draft === true,
    labels,
    headSha,
    ci,
    activeLease: classificationLease || reviewLease,
    canonicalAmbiguous: classifications === null || reviews === null,
    classificationValid,
    reviewOutcome,
    priorReviewTrusted,
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
  lifecycleActor, readLifecycleSnapshot, reconcileLifecycle,
};
