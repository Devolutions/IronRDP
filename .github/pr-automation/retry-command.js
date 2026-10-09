"use strict";

const { CHECKS, canonicalRuns } = require("./automation-lease");
const { readCheckRuns } = require("./check-runs");
const { readLatestExactHeadCiRun, generation } = require("./ci-state");
const { forkRateLimit } = require("./fork-rate-limit");
const { reviewCount } = require("./review-count");
const { reviewPolicyEligible } = require("./routing");
const { parseCheckState } = require("./validate-classifier");
const { SHA, exactKeys } = require("./validation");

const NAME = "AI retry admission";
const VERSION = "retry-admission-v1";
const PREFIX = "ironrdp-pr-automation-retry:";
const COMMAND = "@github-actions retry";
const HOUR = 60 * 60 * 1000;

function commandBody(body) {
  return typeof body === "string" &&
    body.replace(/^[ \t]+|[ \t]+$/g, "") === COMMAND;
}

function positiveId(id) {
  return Number.isSafeInteger(id) && id > 0;
}

function timestamp(value) {
  if (typeof value !== "string" ||
      !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z$/.test(value)) return null;
  const at = Date.parse(value);
  return Number.isFinite(at) && new Date(at).toISOString().slice(0, 19) === value.slice(0, 19)
    ? at : null;
}

function externalId(headSha, stage) {
  return `${VERSION}:${headSha}:${stage}`;
}

function marker({ headSha, stage, commentId, failureCheckId }) {
  return `${PREFIX} ${JSON.stringify({
    schema_version: VERSION, head_sha: headSha, stage, comment_id: commentId,
    failure_check_id: failureCheckId,
  })}`;
}

function parseReceipt(run, headSha, stage) {
  if (run?.name !== NAME || run.app?.slug !== "github-actions" ||
      run.head_sha !== headSha || run.external_id !== externalId(headSha, stage) ||
      run.status !== "completed" || !["success", "neutral"].includes(run.conclusion) ||
      timestamp(run.created_at) === null ||
      typeof run.output?.summary !== "string" ||
      Buffer.byteLength(run.output.summary, "utf8") > 4096) return null;
  const lines = run.output.summary.split(/\r?\n/).map((line) => line.trim())
    .filter((line) => line.startsWith(PREFIX));
  if (lines.length !== 1) return null;
  let receipt;
  try { receipt = JSON.parse(lines[0].slice(PREFIX.length)); } catch { return null; }
  if (!exactKeys(receipt, [
    "schema_version", "head_sha", "stage", "comment_id", "failure_check_id",
  ]) || receipt.schema_version !== VERSION || receipt.head_sha !== headSha ||
      receipt.stage !== stage || !positiveId(receipt.comment_id) ||
      !positiveId(receipt.failure_check_id)) return null;
  return receipt;
}

async function strictRuns(github, owner, repo, headSha, kind) {
  const all = await readCheckRuns({
    github, owner, repo, ref: headSha, checkName: CHECKS[kind].name,
  });
  if (all.some((run) => run?.external_id !== CHECKS[kind].externalId(headSha) ||
      run.head_sha !== headSha || run.app?.slug !== "github-actions")) return null;
  const runs = canonicalRuns(all, { kind, headSha });
  return runs?.every((run) => positiveId(run.id)) ? runs : null;
}

async function admissionHistory(github, owner, repo, headSha) {
  const runs = await readCheckRuns({ github, owner, repo, ref: headSha, checkName: NAME });
  const history = { classification: [], review: [] };
  const ids = new Set();
  const comments = new Set();
  for (const run of runs) {
    // Any check with this name on this SHA must be ours and unambiguous.
    if (run?.head_sha !== headSha || run.app?.slug !== "github-actions") return null;
    const stage = ["classification", "review"]
      .find((candidate) => run.external_id === externalId(headSha, candidate));
    const receipt = stage && parseReceipt(run, headSha, stage);
    if (!receipt || !positiveId(run.id) || ids.has(run.id) ||
        comments.has(receipt.comment_id)) return null;
    ids.add(run.id);
    comments.add(receipt.comment_id);
    history[stage].push({ run, receipt });
  }
  return history;
}

function currentFailure(runs, stage) {
  if (runs.some((run) => run.conclusion === "success")) return null;
  const latest = runs[0];
  if (!latest) return null;
  const title = stage === "review" ? "Automated review unavailable" : "Classification unavailable";
  return latest.status === "completed" && ["failure", "neutral"].includes(latest.conclusion) &&
    latest.output?.title === title
    ? latest : null;
}

async function snapshot({ github, owner, repo, prNumber, commentId, eventHadFailureLabel }) {
  const [{ data: pull }, { data: comment }, { data: issue }] = await Promise.all([
    github.rest.pulls.get({ owner, repo, pull_number: prNumber }),
    github.rest.issues.getComment({ owner, repo, comment_id: commentId }),
    github.rest.issues.get({ owner, repo, issue_number: prNumber }),
  ]);
  const headSha = pull.head?.sha;
  if (!eventHadFailureLabel || pull.number !== prNumber || pull.state !== "open" || pull.draft ||
      !SHA.test(headSha || "") || comment.id !== commentId ||
      comment.issue_url !== pull.issue_url || !commandBody(comment.body) ||
      timestamp(comment.created_at) === null ||
      comment.user?.type !== "User" || !positiveId(comment.user?.id) ||
      pull.user?.type === "Bot" || /\[bot\]$/i.test(pull.user?.login || "") ||
      !issue.labels?.some((label) => (label.name ?? label) === "automation-failed"))
    return null;
  if (comment.user.id !== pull.user?.id) {
    const { data: permission } = await github.rest.repos.getCollaboratorPermissionLevel({
      owner, repo, username: comment.user.login,
    });
    if (!["write", "maintain", "admin"].includes(permission.permission)) return null;
  }
  const classifications = await strictRuns(github, owner, repo, headSha, "classification");
  if (!classifications ||
      classifications.some((run) => run.conclusion === "success" && run !== classifications[0]))
    return null;
  const classification = currentFailure(classifications, "classification");
  // A failed review takes precedence only when a valid current classification exists.
  const currentClass = classifications[0];
  const classValid = currentClass?.status === "completed" &&
    currentClass.conclusion === "success" && currentClass.output?.title === "Classification complete" &&
    parseCheckState(currentClass.output?.summary)?.automaticReviewEligible === true;
  const stage = classification ? "classification" : classValid ? "review" : null;
  if (!stage) return null;
  const reviews = await strictRuns(github, owner, repo, headSha, "review");
  if (!reviews || (stage === "classification" && reviews.length > 0)) return null;
  const failure = stage === "classification"
    ? classification : currentFailure(reviews, "review");
  if (!failure || timestamp(failure.completed_at) === null ||
      timestamp(comment.created_at) <= timestamp(failure.completed_at)) return null;
  let gate = stage === "classification" ? null : { classificationId: currentClass.id };
  if (stage === "review") {
    const labels = issue.labels.map((label) => label.name ?? label);
    if (!reviewPolicyEligible({ labels }) || reviewCount(labels) === undefined ||
        !classValid)
      return null;
    // A published bot review on this head may have escaped a failed check.
    for await (const page of github.paginate.iterator(github.rest.pulls.listReviews, {
      owner, repo, pull_number: prNumber, per_page: 100,
    })) {
      if (!Array.isArray(page.data) || page.data.some((review) =>
        review.user?.login === "github-actions[bot]" && review.commit_id === headSha)) return null;
    }
    const ci = await readLatestExactHeadCiRun({
      github, owner, repo, expectedSha: headSha, retries: 0,
    });
    const ciGeneration = generation(ci);
    if (ci?.status !== "completed" || ci.conclusion !== "success" || !ciGeneration) return null;
    gate = { ...gate, ciGeneration, count: reviewCount(labels) };
  }
  const limit = await forkRateLimit({ github, owner, repo, pr: pull });
  if (limit.status !== "allowed") return null;
  const history = await admissionHistory(github, owner, repo, headSha);
  if (!history) return null;
  return { headSha, stage, failureCheckId: failure.id, commentCreatedAt: comment.created_at, history, gate };
}

async function acceptRetry({ github, owner, repo, prNumber, commentId, eventHadFailureLabel }) {
  if (!positiveId(prNumber) || !positiveId(commentId) || eventHadFailureLabel !== true) return false;
  const read = async () => {
    try { return await snapshot({ github, owner, repo, prNumber, commentId, eventHadFailureLabel }); }
    catch { return null; }
  };
  const initial = await read();
  if (!initial) return false;
  const current = await read();
  const same = (left, right) => left && right && left.headSha === right.headSha &&
    left.stage === right.stage && left.failureCheckId === right.failureCheckId &&
    left.commentCreatedAt === right.commentCreatedAt &&
    JSON.stringify(left.gate) === JSON.stringify(right.gate);
  if (!same(current, initial))
    return false;
  const history = current.history[current.stage];
  if (Object.values(current.history).flat().some(({ receipt }) => receipt.comment_id === commentId))
    return false;
  const accepted = history.filter(({ run }) => run.conclusion === "success");
  const last = Math.max(...accepted.map(({ run }) => timestamp(run.created_at)), -Infinity);
  const time = timestamp(current.commentCreatedAt);
  if (accepted.length >= 3 || time < last + HOUR) return false;
  const output = { title: "Retry admitted", summary: marker({ ...current, commentId }) };
  const { data: admission } = await github.rest.checks.create({
    owner, repo, name: NAME, head_sha: current.headSha,
    external_id: externalId(current.headSha, current.stage),
    status: "completed", conclusion: "success", output,
  });
  const neutralize = async () => github.rest.checks.update({
    owner, repo, check_run_id: admission.id, status: "completed",
    conclusion: "neutral", output: { ...output, title: "Retry not dispatched" },
  });
  // The receipt is durable before dispatch: a crash here counts one attempt, never two.
  const final = await read();
  if (!same(final, current) ||
      !final.history[current.stage].some(({ run }) => run.id === admission.id)) {
    await neutralize();
    return false;
  }
  // Dispatch errors may follow server acceptance: retain the receipt and fail visibly.
  await github.rest.actions.createWorkflowDispatch({
    owner, repo, workflow_id: "pr-automation.yml", ref: "master",
    inputs: {
      "pr-number": String(prNumber), review: String(current.stage === "review"), force: "false",
      "retry-head-sha": current.headSha,
      "retry-failure-check-id": String(current.failureCheckId),
      "retry-admission-check-id": String(admission.id),
    },
  });
  return true;
}

function retryBinding(inputs = {}) {
  const keys = ["retry-head-sha", "retry-failure-check-id", "retry-admission-check-id"];
  const values = keys.map((key) => inputs[key]);
  if (values.every((value) => value === undefined || value === "")) return null;
  if (!SHA.test(values[0] || "") ||
      values.slice(1).some((value) => typeof value !== "string" ||
        !/^[1-9]\d*$/.test(value) || !positiveId(Number(value)))) return false;
  return { headSha: values[0], failureCheckId: Number(values[1]), admissionCheckId: Number(values[2]) };
}

async function validRetryBinding({ github, owner, repo, prNumber, stage, binding }) {
  if (!binding || !positiveId(prNumber) || !["classification", "review"].includes(stage)) return false;
  try {
    const history = await admissionHistory(github, owner, repo, binding.headSha);
    const admitted = history?.[stage].find(({ run }) => run.id === binding.admissionCheckId);
    if (!admitted || admitted.run.conclusion !== "success" ||
        admitted.receipt.failure_check_id !== binding.failureCheckId) return false;
    const state = await snapshot({
      github, owner, repo, prNumber, commentId: admitted.receipt.comment_id,
      eventHadFailureLabel: true,
    });
    return state?.headSha === binding.headSha && state.stage === stage &&
      state.failureCheckId === binding.failureCheckId &&
      state.history[stage].some(({ run, receipt }) =>
        run.id === binding.admissionCheckId && receipt.comment_id === admitted.receipt.comment_id);
  } catch {
    return false;
  }
}

module.exports = {
  acceptRetry, commandBody, marker, parseReceipt, admissionHistory, retryBinding, validRetryBinding,
};
