"use strict";

const { SCHEMA_VERSION, parseCheckState } = require("./validate-classifier");
const { contributorEligibility, reviewPolicyEligible } = require("./resolve-state");
const { forkRateLimit } = require("./fork-rate-limit");
const { normalizeReviewerIds } = require("./routing");
const { isRetryableFailure, plannedRequiredReviewers } = require("./review-pipeline");

const MAXIMUM_DELAY_SECONDS = 15 * 60;
const OVERSIZED_REVIEW_LABEL = "ai-review/allow-oversized";
const MIB = 1024 * 1024;
// A classification that stopped the automation keeps its eligibility flag, so only this title tells
// the two apart. The caller's own gate reads it the same way.
const CLASSIFICATION_COMPLETE = "Classification complete";

const decline = (reason) => ({ retry: false, reason });

async function assertCurrentHead({ github, owner, repo, pullNumber, expectedHeadSha }) {
  const { data } = await github.rest.pulls.get({ owner, repo, pull_number: pullNumber });
  if (data.state !== "open" || data.head?.sha !== expectedHeadSha) {
    throw new Error("pull request head is no longer current");
  }
}

function labelNames(labels) {
  return (labels || []).map((label) => typeof label === "string" ? label : label?.name)
    .filter(Boolean);
}

function sameReviewers(left, right) {
  const a = normalizeReviewerIds(left);
  const b = normalizeReviewerIds(right);
  return Boolean(a && b && a.length === b.length && a.every((id, index) => id === b[index]));
}

// The delay is long enough for the pull request to stop deserving the review a retry would finish,
// so every caller gate decision is made again against the current pull request.
async function retryStillPermitted({
  github, owner, repo, pullNumber, expectedHeadSha, expectedBaseSha, force = false,
  selectedReviewers = [], requiredReviewers, diffBytes = null,
} = {}) {
  const pull = (await github.rest.pulls.get({ owner, repo, pull_number: pullNumber })).data;
  if (pull.state !== "open") return decline("pull request is no longer open");
  if (pull.head?.sha !== expectedHeadSha) return decline("pull request head is no longer current");
  if (expectedBaseSha && pull.base?.sha !== expectedBaseSha) {
    return decline("pull request base moved away from the reviewed evidence");
  }

  const labels = labelNames(pull.labels);

  // A withdrawn oversized allowance shrinks the cap the evidence was fetched under. Evidence that
  // cannot be measured cannot be shown to fit, so it does not get a second request.
  if (diffBytes === null) return decline("pull request evidence is unavailable");
  const cap = labels.includes(OVERSIZED_REVIEW_LABEL) ? 4 * MIB : MIB;
  if (diffBytes > cap) return decline("pull request evidence exceeds the current evidence limit");

  // A trusted caller's force bypasses policy and CI, and nothing else.
  if (force) return { retry: true, reason: "" };

  if (pull.user?.type === "Bot") return decline("pull request author is a bot");
  if (pull.draft === true) return decline("pull request is a draft");
  if (!reviewPolicyEligible({ labels })) return decline("review is no longer policy eligible");

  const runsFor = async (checkName) => (await github.rest.checks.listForRef({
    owner, repo, ref: expectedHeadSha, check_name: checkName, per_page: 100,
  })).data.check_runs.filter((run) => run?.app?.slug === "github-actions");

  const classification = (await runsFor("AI classification"))
    .find((run) => run.external_id === `${SCHEMA_VERSION}:${expectedHeadSha}`);
  if (classification?.conclusion !== "success") {
    return decline("classification is no longer valid for this head");
  }
  const state = parseCheckState(classification.output?.summary);
  if (state === null || state.automaticReviewEligible !== true ||
      classification.output?.title !== CLASSIFICATION_COMPLETE) {
    return decline("classification no longer authorizes an automatic review");
  }
  if (!sameReviewers(state.specialistReviewers, selectedReviewers)) {
    return decline("classification now selects a different reviewer set");
  }
  const stillSelected = normalizeReviewerIds(state.specialistReviewers) || [];
  if (Array.isArray(requiredReviewers) &&
      requiredReviewers.some((reviewer) => !stillSelected.includes(reviewer))) {
    return decline("a required reviewer is no longer selected");
  }

  if ((await runsFor("AI automated review")).some((run) => run.conclusion === "success")) {
    return decline("this head was already reviewed");
  }

  // The newest CI run decides. An older success cannot vouch for a rerun that is failing or still
  // in flight.
  const ciRuns = (await github.rest.actions.listWorkflowRunsForRepo({
    owner, repo, head_sha: expectedHeadSha, per_page: 100,
  })).data.workflow_runs.filter((run) => run?.name === "CI");
  const latestCi = ciRuns.reduce((newest, run) =>
    !newest || Date.parse(run.run_started_at ?? run.created_at ?? 0) >=
      Date.parse(newest.run_started_at ?? newest.created_at ?? 0) ? run : newest, null);
  if (latestCi?.conclusion !== "success") return decline("CI is not green at the reviewed head");

  const contributor = await contributorEligibility({
    github, owner, repo, currentPrNumber: pullNumber,
    author: {
      association: pull.author_association,
      login: pull.user?.login,
      nodeId: pull.user?.node_id,
      type: pull.user?.type,
    },
  });
  if (contributor.status !== "eligible") return decline("pull request author is no longer eligible");

  const quota = await forkRateLimit({ github, owner, repo, pr: pull });
  if (quota.status !== "allowed") return decline("fork review quota is exhausted");

  return { retry: true, reason: "" };
}

// A retry costs a provider request, so the runtime must have called the failure retryable and the
// review must still be publishable after the delay.
async function delayedRetryGate({
  retryable, failureCategory, delaySeconds,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  ...policy
} = {}) {
  if (!isRetryableFailure(retryable)) {
    return decline(`${failureCategory || "the failure"} is not retryable`);
  }
  const delay = Number.isFinite(delaySeconds) && delaySeconds >= 0
    ? Math.min(delaySeconds, MAXIMUM_DELAY_SECONDS)
    : 0;
  await sleep(delay * 1000);
  try {
    return await retryStillPermitted(policy);
  } catch {
    // Failing to prove the review is still wanted is not permission to spend another request.
    return decline("review eligibility could not be confirmed");
  }
}

// Both reviewer jobs decide a retry the same way and differ only in the stage they log, so the
// step keeps the decision here rather than repeating it in YAML.
async function retryGateStep({ github, context, core, env, stage }) {
  const diffBytes = (() => {
    try {
      return require("node:fs").statSync("pr-evidence/pull-request.diff").size;
    } catch {
      return null;
    }
  })();
  const force = (() => {
    try {
      return JSON.parse(env.GATE || "{}")?.force === true;
    } catch {
      return false;
    }
  })();
  const selectedReviewers = plannedRequiredReviewers(env.SELECTED_REVIEWERS);
  const decision = await delayedRetryGate({
    github, owner: context.repo.owner, repo: context.repo.repo,
    pullNumber: Number(env.PULL_REQUEST_NUMBER),
    expectedHeadSha: env.HEAD_SHA,
    expectedBaseSha: env.BASE_SHA,
    retryable: env.RETRYABLE,
    failureCategory: env.FAILURE_CATEGORY,
    delaySeconds: Number(env.RETRY_DELAY_SECONDS),
    force,
    selectedReviewers,
    requiredReviewers: plannedRequiredReviewers(env.REQUIRED_REVIEWERS, selectedReviewers),
    diffBytes,
  });
  core.setOutput("retry", String(decision.retry));
  core.setOutput("reason", decision.reason);
  core.info(JSON.stringify({ event: "pr-automation.retry-gate", stage, ...decision }));
  return decision;
}

module.exports = {
  MAXIMUM_DELAY_SECONDS, assertCurrentHead, delayedRetryGate, retryGateStep, retryStillPermitted,
};
