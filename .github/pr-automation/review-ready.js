"use strict";

const { canonicalRuns } = require("./automation-lease");
const { readCheckRuns } = require("./check-runs");
const { trustedReviewReceipt } = require("./review-outcome");
const { reviewCount } = require("./review-count");
const { exactKeys, SHA } = require("./validation");

const NAME = "AI review-ready";
const PREFIX = "ironrdp-pr-automation-review-ready:";
const VERSION = "review-ready-v1";
const BOT = "github-actions[bot]";
const COMMAND = "@github-actions review-ready";

function commandBody(body) {
  return typeof body === "string" &&
    body.replace(/^[ \t]+|[ \t]+$/g, "") === COMMAND;
}

function positiveId(id) {
  return Number.isSafeInteger(id) && id > 0;
}

function serverSecond(value) {
  if (typeof value !== "string" ||
      !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z$/.test(value)) return null;
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds) ||
      new Date(milliseconds).toISOString().slice(0, 19) !== value.slice(0, 19)) return null;
  return Math.floor(milliseconds / 1000);
}

function encodeReady({ headSha, commentId, reviewId, checkId }) {
  if (!SHA.test(headSha) ||
      ![commentId, reviewId, checkId].every(positiveId)) throw new Error("invalid review-ready receipt");
  return `${PREFIX} ${JSON.stringify({
    schema_version: VERSION, head_sha: headSha, comment_id: commentId,
    review_id: reviewId, review_check_id: checkId,
  })}`;
}

function trustedReady(run, headSha) {
  if (run?.app?.slug !== "github-actions" || run.name !== NAME ||
      run.head_sha !== headSha || run.external_id !== headSha ||
      run.status !== "completed" || run.conclusion !== "success" ||
      serverSecond(run.created_at) === null ||
      typeof run.output?.summary !== "string" ||
      Buffer.byteLength(run.output.summary, "utf8") > 4096) return null;
  const lines = run.output.summary.split(/\r?\n/).map((line) => line.trim())
    .filter((line) => line.startsWith(PREFIX));
  if (lines.length !== 1) return null;
  let receipt;
  try { receipt = JSON.parse(lines[0].slice(PREFIX.length)); } catch { return null; }
  if (!exactKeys(receipt, ["schema_version", "head_sha", "comment_id", "review_id", "review_check_id"]) ||
      receipt.schema_version !== VERSION || receipt.head_sha !== headSha ||
      ![receipt.comment_id, receipt.review_id, receipt.review_check_id].every(positiveId)) return null;
  return receipt;
}

async function publishedReview(github, owner, repo, prNumber) {
  const reviews = [];
  for await (const page of github.paginate.iterator(github.rest.pulls.listReviews, {
    owner, repo, pull_number: prNumber, per_page: 100,
  })) reviews.push(...page.data);
  const botReviews = reviews.filter((review) => review.user?.login === BOT);
  if (botReviews.some((review) => !positiveId(review.id))) return null;
  return botReviews.sort((left, right) => right.id - left.id)[0] ?? null;
}

async function authoritativeFindings({ github, owner, repo, prNumber, headSha, reviewRuns, labels }) {
  const runs = reviewRuns ?? canonicalRuns(await readCheckRuns({
    github, owner, repo, ref: headSha, checkName: "AI automated review",
  }), { kind: "review", headSha });
  if (!runs) return null;
  const current = runs[0];
  const receipt = trustedReviewReceipt(current, headSha);
  if (receipt?.outcome !== "findings" || !positiveId(current.id) ||
      (labels && reviewCount(labels) !== receipt.next_review_count)) return null;
  const review = await publishedReview(github, owner, repo, prNumber);
  if (!review || review.commit_id !== headSha ||
      typeof review.body !== "string" ||
      !receipt.review_marker ||
      !(review.body === receipt.review_marker ||
        review.body.startsWith(`${receipt.review_marker}\n`))) return null;
  return { checkId: current.id, reviewId: review.id,
    reviewSecond: serverSecond(review.submitted_at) };
}

async function readyRuns(github, owner, repo, headSha) {
  const runs = await readCheckRuns({ github, owner, repo, ref: headSha, checkName: NAME });
  const matching = runs.filter((run) => run.external_id === headSha);
  if (matching.length > 1) return null;
  const existing = matching[0] ?? null;
  const receipt = existing && trustedReady(existing, headSha);
  return existing && !receipt ? null : { existing, receipt };
}

async function reviewReadyAcknowledged({ github, owner, repo, prNumber, headSha, reviewRuns, labels }) {
  const ready = await readyRuns(github, owner, repo, headSha);
  if (!ready?.receipt) return false;
  const authority = await authoritativeFindings({
    github, owner, repo, prNumber, headSha, reviewRuns, labels,
  });
  return authority?.checkId === ready.receipt.review_check_id &&
    authority?.reviewId === ready.receipt.review_id;
}

async function acceptReviewReady({ github, owner, repo, prNumber, commentId }) {
  if (!positiveId(commentId) || !positiveId(prNumber)) return false;
  const read = async () => {
    const [{ data: pull }, { data: comment }, { data: issue }] = await Promise.all([
      github.rest.pulls.get({ owner, repo, pull_number: prNumber }),
      github.rest.issues.getComment({ owner, repo, comment_id: commentId }),
      github.rest.issues.get({ owner, repo, issue_number: prNumber }),
    ]);
    if (pull.state !== "open" || pull.draft || pull.number !== prNumber ||
        comment.id !== commentId ||
        comment.issue_url !== pull.issue_url || !commandBody(comment.body) ||
        comment.user?.type !== "User" || !positiveId(comment.user?.id) ||
        !SHA.test(pull.head?.sha || "")) return null;
    if (comment.user.id !== pull.user?.id) {
      let permission;
      try {
        ({ data: { permission } } = await github.rest.repos.getCollaboratorPermissionLevel({
          owner, repo, username: comment.user.login,
        }));
      } catch { return null; }
      if (!["write", "maintain", "admin"].includes(permission)) return null;
    }
    const headSha = pull.head.sha;
    const authoritative = await authoritativeFindings({
      github, owner, repo, prNumber, headSha, labels: issue.labels,
    });
    const commentSecond = serverSecond(comment.created_at);
    if (!authoritative || commentSecond === null ||
        authoritative.reviewSecond === null ||
        commentSecond <= authoritative.reviewSecond) return null;
    const ready = await readyRuns(github, owner, repo, headSha);
    if (!ready) return null;
    return { headSha, authoritative, ...ready };
  };
  const initial = await read();
  if (!initial) return false;
  // Repeat all authority reads at the mutation boundary, including permission and review identity.
  const current = await read();
  if (!current || current.headSha !== initial.headSha ||
      current.authoritative.reviewId !== initial.authoritative.reviewId ||
      current.authoritative.checkId !== initial.authoritative.checkId ||
      current.existing?.id !== initial.existing?.id) return false;
  if (current.receipt?.review_id === current.authoritative.reviewId &&
      current.receipt?.review_check_id === current.authoritative.checkId) return false;
  const summary = encodeReady({
    headSha: current.headSha, commentId, reviewId: current.authoritative.reviewId,
    checkId: current.authoritative.checkId,
  });
  const output = { title: "Review ready for human review", summary };
  if (current.existing) {
    await github.rest.checks.update({
      owner, repo, check_run_id: current.existing.id, status: "completed", conclusion: "success",
      output,
    });
  } else {
    await github.rest.checks.create({
      owner, repo, name: NAME, head_sha: current.headSha, external_id: current.headSha,
      status: "completed", conclusion: "success",
      output,
    });
  }
  return true;
}

module.exports = {
  commandBody, encodeReady, trustedReady, publishedReview,
  readyRuns, reviewReadyAcknowledged, acceptReviewReady,
};
