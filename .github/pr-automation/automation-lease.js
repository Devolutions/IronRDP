"use strict";

const { SCHEMA_VERSION } = require("./validate-classifier");
const { readCheckRuns } = require("./check-runs");
const { matchesGeneration, readLatestExactHeadCiRun } = require("./ci-state");
const { reviewCount } = require("./resolve-state");
const { reviewPolicyEligible } = require("./routing");
const { isClosedUnmerged, isOpenNonDraftAtHeadNow } = require("./current-head");
const { parseCheckState } = require("./validate-classifier");

const APP = "github-actions";
const LEASE_PREFIX = "<!-- ironrdp-pr-automation:lease:";
const CHECKS = {
  classification: { name: "AI classification", externalId: (sha) => `${SCHEMA_VERSION}:${sha}` },
  review: { name: "AI automated review", externalId: (sha) => sha },
};
function leaseMarker({ kind, headSha, runId, attempt }) {
  return `${LEASE_PREFIX}v1:${kind}:${headSha}:${runId}:${attempt} -->`;
}

function parseLeaseMarker(summary) {
  const match = new RegExp(`^${LEASE_PREFIX}v1:(classification|review):([0-9a-f]{40}):(\\d+):(\\d+) -->$`).exec(summary || "");
  if (!match || !Number.isSafeInteger(Number(match[3])) || Number(match[3]) <= 0 ||
      !Number.isSafeInteger(Number(match[4])) || Number(match[4]) <= 0) {
    return null;
  }
  return { kind: match[1], headSha: match[2], runId: Number(match[3]), attempt: Number(match[4]) };
}
function canonicalRuns(runs, { kind, headSha }) {
  const spec = CHECKS[kind];
  const externalId = spec.externalId(headSha);
  const matching = runs.filter((run) => run.external_id === externalId);
  if (matching.some((run) => run.app?.slug !== APP || run.head_sha !== headSha)) return null;
  return matching.sort((left, right) => Number(right.id) - Number(left.id));
}
async function readCanonicalRuns({ github, owner, repo, kind, headSha }) {
  const spec = CHECKS[kind];
  return canonicalRuns(await readCheckRuns({
    github, owner, repo, ref: headSha, checkName: spec.name,
  }), { kind, headSha });
}
async function ownerIsActive(github, lease) {
  try {
    const { data: run } = await github.rest.actions.getWorkflowRun({
      owner: lease.owner, repo: lease.repo, run_id: lease.runId,
    });
    return run.run_attempt === lease.attempt && run.status !== "completed";
  } catch (error) {
    if (error?.status === 404) return false;
    throw error;
  }
}
async function claimAutomaticLease({
  github, owner, repo, kind, headSha, runId, attempt, allowSuccess = false, prNumber, gate,
}) {
  const eligible = async (lease) => {
    if (prNumber === undefined) return true;
    if (kind === "review") {
      return await recheckAutomaticReview({
        github, owner, repo, prNumber, headSha, gate, lease,
        phase: lease ? "admission" : "preclaim",
      });
    }
    return await isOpenNonDraftAtHeadNow({
      github, owner, repo, pullNumber: prNumber, expectedHeadSha: headSha,
    });
  };
  if (prNumber !== undefined) {
    if (!Number.isSafeInteger(prNumber) || prNumber <= 0) {
      return { owner: false, available: false, reason: "pull request identity is unavailable" };
    }
  }
  const runs = await readCanonicalRuns({ github, owner, repo, kind, headSha });
  if (!runs) return { owner: false, available: false, reason: "canonical check is ambiguous" };
  if (!allowSuccess && runs.some((run) => run.conclusion === "success")) {
    return { owner: false, available: true, reason: "canonical success exists" };
  }
  const latest = runs[0];
  if (latest?.status === "in_progress") {
    const lease = parseLeaseMarker(latest.output?.summary);
    if (!lease || lease.kind !== kind || lease.headSha !== headSha) {
      return { owner: false, available: false, reason: "canonical lease is ambiguous" };
    }
    if (await ownerIsActive(github, { ...lease, owner, repo })) {
      return { owner: false, available: true, reason: "canonical lease is active" };
    }
  }
  const marker = leaseMarker({ kind, headSha, runId, attempt });
  const spec = CHECKS[kind];
  // Re-read immediately before the consequential claim rather than relying on the admission read.
  if (!await eligible()) {
    return { owner: false, available: true, reason: "pull request is no longer eligible" };
  }
  const { data: claim } = await github.rest.checks.create({
    owner, repo, name: spec.name, head_sha: headSha, external_id: spec.externalId(headSha),
    status: "in_progress", output: { title: "Automation in progress", summary: marker },
  });
  const lease = { kind, headSha, runId, attempt, checkRunId: claim.id, marker };
  // An eligibility change during `create` makes this claim unusable. Complete only our new claim
  // so a later admission may proceed without inheriting an active-looking lease.
  if (!await eligible(lease)) {
    await github.rest.checks.update({
      owner, repo, check_run_id: claim.id, status: "completed", conclusion: "neutral",
      output: { title: "Automation superseded", summary: marker },
    });
    return { owner: false, available: true, reason: "pull request changed during lease claim" };
  }
  return {
    owner: true, available: true,
    lease,
  };
}

async function ownsLatestLease({ github, owner, repo, lease }) {
  if (!lease || lease.marker !== leaseMarker(lease)) return false;
  const runs = await readCanonicalRuns({
    github, owner, repo, kind: lease.kind, headSha: lease.headSha,
  });
  return Array.isArray(runs) && runs[0]?.id === lease.checkRunId &&
    runs[0].status === "in_progress" &&
    runs[0].output?.summary === lease.marker;
}

async function ownsActiveLease({ github, owner, repo, lease }) {
  return await ownsLatestLease({ github, owner, repo, lease }) && await ownerIsActive(github, { ...lease, owner, repo });
}

async function hasCurrentClassification({ github, owner, repo, headSha, classificationId, requireValid = false }) {
  const classifications = await readCanonicalRuns({
    github, owner, repo, kind: "classification", headSha,
  });
  const classification = classifications?.[0];
  return classification?.id === classificationId && classification?.conclusion === "success" &&
    (!requireValid || parseCheckState(classification.output?.summary) !== null);
}

async function recheckAutomaticReview({
  github, owner, repo, prNumber, headSha, gate, lease, phase = "admission",
  allowDraft = false, allowClosedUnmerged = false,
}) {
  if (![
    "preclaim", "admission", "transition-old", "transition-empty", "transition-next",
    "receipt", "cleanup",
  ].includes(phase)) throw new Error("invalid review recheck phase");
  const { data: pull } = await github.rest.pulls.get({ owner, repo, pull_number: prNumber });
  if ((pull.state !== "open" && !(allowClosedUnmerged && isClosedUnmerged(pull))) ||
      (!allowDraft && pull.draft) || pull.head?.sha !== headSha) return false;
  const reviews = await readCanonicalRuns({ github, owner, repo, kind: "review", headSha });
  if (!reviews || !await hasCurrentClassification({
    github, owner, repo, headSha, classificationId: gate.classificationId,
  })) return false;
  if (phase === "cleanup") {
    if (reviews[0]?.id !== lease?.checkRunId || reviews[0]?.conclusion !== "success") return false;
  } else if (phase !== "preclaim") {
    if (!lease || !await ownsActiveLease({ github, owner, repo, lease })) return false;
  }
  if (phase !== "cleanup" && reviews.some((run) => run.conclusion === "success")) return false;
  const currentCi = await readLatestExactHeadCiRun({
    github, owner, repo, expectedSha: headSha,
    expectedGeneration: { id: gate.ciRunId, attempt: gate.ciRunAttempt },
  });
  if (currentCi?.conclusion !== "success" ||
      !matchesGeneration(currentCi, gate.ciRunId, gate.ciRunAttempt)) return false;
  const { data: issue } = await github.rest.issues.get({ owner, repo, issue_number: prNumber });
  const labels = issue.labels.map((label) => typeof label === "string" ? label : label.name);
  const count = reviewCount(labels);
  if (count === undefined) return false;
  if (phase === "transition-empty") {
    return count === null &&
      reviewPolicyEligible({ labels, legitimacyStopped: labels.includes("triage/legitimacy") }) ===
        gate.policyEligible &&
      labels.includes("triage/legitimacy") === gate.legitimacyStopped;
  }
  if (["transition-next", "receipt", "cleanup"].includes(phase)) {
    return count === gate.nextCount;
  }
  const admittedCount = reviewCount(gate.labels || []);
  return admittedCount !== undefined && count === admittedCount &&
    reviewPolicyEligible({ labels, legitimacyStopped: labels.includes("triage/legitimacy") }) ===
      gate.policyEligible &&
    labels.includes("triage/legitimacy") === gate.legitimacyStopped;
}

module.exports = {
  CHECKS, canonicalRuns, claimAutomaticLease, leaseMarker, ownerIsActive,
  ownsActiveLease, ownsLatestLease,
  hasCurrentClassification, parseLeaseMarker, recheckAutomaticReview,
};
