"use strict";

const { encodeCheckState } = require("./validate-classifier");
const { provenancePrefix } = require("./validate-final-review");
const { reviewPolicyEligible } = require("./routing");

class StaleHeadError extends Error {
  constructor() { super("pull request head is no longer current"); this.name = "StaleHeadError"; }
}

class StalePolicyError extends Error {
  constructor() { super("pull request review policy changed"); this.name = "StalePolicyError"; }
}

// Model output is treated as hostile, so it is neutralized before it reaches a bot-authored
// comment or review. HTML, code spans, mentions, and issue references are defused, and the
// Markdown constructs that would otherwise still render as active links, images, or formatting are
// backslash-escaped so that text such as `[label](https://example.invalid)` stays inert prose.
function escapeMarkdown(value) {
  return String(value).replace(/\\/g, "\\\\")
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;").replace(/'/g, "&#39;").replace(/`/g, "&#96;")
    .replace(/@(?=[\w-])/g, "`@`").replace(/(?<!&)#(?=\d)/g, "`#`")
    .replace(/[[\]()!*_~|]/g, "\\$&");
}

async function assertCurrentHead(github, owner, repo, prNumber, expectedSha) {
  const { data: pr } = await github.rest.pulls.get({ owner, repo, pull_number: prNumber });
  if (pr.state !== "open" || pr.head?.sha !== expectedSha) throw new StaleHeadError();
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
  if (comment.kind === "duplicate") {
    return `${comment.marker}\n\nPotential duplicate detected: ${escapeMarkdown(comment.url)}.\n\n${escapeMarkdown(comment.rationale)}\n\nMaintainer review is required.`;
  }
  if (comment.kind === "legitimacy") {
    return `${comment.marker}\n\nAutomated review stopped because commit \`${escapeMarkdown(comment.sha)}\` has strong indicators requiring maintainer triage.\n\n${escapeMarkdown(comment.reason)}\n\nThis comment remains as an audit record if later classifications differ. Maintainer review is required.`;
  }
  if (comment.kind === "global-quota") {
    return `${comment.marker}\n\nAutomated classification and review capacity for fork pull requests has reached its daily UTC limit.\n\nSee the [automation policy](https://github.com/${owner}/${repo}/blob/master/.github/PR_AUTOMATION.md). Maintainer review is required.`;
  }
  if (comment.kind === "evidence-limit") {
    const guidance = comment.limitMiB === 1
      ? "A maintainer can add `ai-review/allow-oversized` to retry with the runtime maximum of 4 MiB. Otherwise, split the change into focused pull requests or reduce generated content."
      : "The 4 MiB limit is the model runtime maximum. Please split the change into focused pull requests or reduce generated content.";
    return `${comment.marker}\n\nAutomated model analysis stopped because this pull request's diff exceeds the ${comment.limitMiB} MiB evidence limit. No model was invoked with partial evidence.\n\n${guidance} Maintainer review is required.`;
  }
  throw new Error("unsupported issue comment");
}

async function upsertMarkedComment(github, owner, repo, prNumber, expectedSha, botLogin, comment) {
  if (!botLogin || typeof botLogin !== "string") throw new Error("botLogin is required for comment ownership");
  const body = markerBody(comment, owner, repo);
  const existing = (await comments(github, owner, repo, prNumber)).find((item) =>
    item.user?.login === botLogin && typeof item.body === "string" && item.body.includes(comment.marker));
  if (existing?.body === body) return false;
  await issueLabels(github, owner, repo, prNumber);
  await assertCurrentHead(github, owner, repo, prNumber, expectedSha);
  if (existing) {
    await github.rest.issues.updateComment({ owner, repo, comment_id: existing.id, body });
  } else {
    await github.rest.issues.createComment({ owner, repo, issue_number: prNumber, body });
  }
  return true;
}

async function deleteMarkedComment(github, owner, repo, prNumber, expectedSha, botLogin, marker) {
  if (!botLogin || typeof botLogin !== "string") throw new Error("botLogin is required for comment ownership");
  const existing = (await comments(github, owner, repo, prNumber)).find((item) =>
    item.user?.login === botLogin && typeof item.body === "string" && item.body.includes(marker));
  if (!existing) return false;
  await assertCurrentHead(github, owner, repo, prNumber, expectedSha);
  await github.rest.issues.deleteComment({ owner, repo, comment_id: existing.id });
  return true;
}

function reviewBody(marker, review) {
  const findings = review.findings.filter((finding) => finding.start_line === null).map((finding, index) => {
    return `${index + 1}. **${provenancePrefix(finding.sources)} ${escapeMarkdown(finding.title)}** — ` +
      `${finding.classification} / ${finding.severity} — ${escapeMarkdown(finding.path)}\n` +
      `   ${escapeMarkdown(finding.rationale)}`;
  }).join("\n");
  return `${marker}\n\n${escapeMarkdown(review.summary)}${findings ? `\n\n${findings}` : ""}`;
}

async function reviews(github, owner, repo, prNumber) {
  const result = [];
  for await (const response of github.paginate.iterator(github.rest.pulls.listReviews, {
    owner, repo, pull_number: prNumber, per_page: 100,
  })) result.push(...response.data);
  return result;
}

function assertReviewPolicy(labels, state) {
  const currentReviewCount = labels.has("ai-reviewed/2") ? "ai-reviewed/2"
    : labels.has("ai-reviewed/1") ? "ai-reviewed/1"
    : null;
  if (currentReviewCount !== state.expectedReviewCount ||
      (!state.forced && !reviewPolicyEligible({
        labels: [...labels],
      }))) {
    throw new StalePolicyError();
  }
}

async function publishReview(github, owner, repo, prNumber, state, botLogin, comment) {
  if (!botLogin || typeof botLogin !== "string") throw new Error("botLogin is required for review ownership");
  if ((await reviews(github, owner, repo, prNumber)).some((review) =>
    review.user?.login === botLogin && typeof review.body === "string" && review.body.includes(comment.marker))) return false;
  const review = comment.review;
  const inline = review.findings.filter((finding) => finding.start_line !== null).map((finding) => {
    const comment = {
      path: finding.path, line: finding.end_line, side: "RIGHT",
      body: `**${provenancePrefix(finding.sources)} ${escapeMarkdown(finding.title)}** — ` +
        `${finding.classification} / ${finding.severity}: ${escapeMarkdown(finding.rationale)}`,
    };
    if (finding.start_line !== finding.end_line) {
      comment.start_line = finding.start_line;
      comment.start_side = "RIGHT";
    }
    return comment;
  });
  await assertCurrentHead(github, owner, repo, prNumber, state.expectedSha);
  assertReviewPolicy(await issueLabels(github, owner, repo, prNumber), state);
  await github.rest.pulls.createReview({
    owner, repo, pull_number: prNumber, commit_id: state.expectedSha, event: "COMMENT",
    body: reviewBody(comment.marker, review), comments: inline,
  });
  return true;
}

async function findCheck(github, owner, repo, expectedSha, check) {
  let found = null;
  for await (const response of github.paginate.iterator(github.rest.checks.listForRef, {
    owner, repo, ref: expectedSha, check_name: check.name, per_page: 100,
  })) {
    for (const run of response.data) {
      if (run.external_id === check.externalId && (!found || run.id > found.id)) found = run;
    }
  }
  return found;
}

async function ensureClassificationCheck(github, owner, repo, prNumber, expectedSha, check) {
  const summary = `${check.summary}\n\n${encodeCheckState(check.machineState)}`;
  const conclusion = check.conclusion ?? "success";
  const existing = await findCheck(github, owner, repo, expectedSha, check);
  if (existing?.conclusion === conclusion && existing.output?.title === check.title &&
      existing.output?.summary === summary) return false;
  await assertCurrentHead(github, owner, repo, prNumber, expectedSha);
  const payload = {
    owner, repo, name: check.name, head_sha: expectedSha, external_id: check.externalId,
    status: "completed", conclusion,
    output: { title: check.title, summary },
  };
  if (existing) {
    await github.rest.checks.update({ ...payload, check_run_id: existing.id });
  } else {
    await github.rest.checks.create(payload);
  }
  return true;
}

async function ensureReviewCheck(github, owner, repo, prNumber, expectedSha, check, state) {
  const conclusion = check.conclusion ?? "success";
  const title = check.title ?? "Automated review complete";
  const summary = check.summary ?? "Validated automated review is bound to this commit.";
  const existing = await findCheck(github, owner, repo, expectedSha, check);
  if (existing?.conclusion === conclusion && existing.output?.title === title &&
      existing.output?.summary === summary) return false;
  if (state.failed !== true) {
    assertReviewPolicy(await issueLabels(github, owner, repo, prNumber), state);
  }
  await assertCurrentHead(github, owner, repo, prNumber, expectedSha);
  const payload = {
    owner, repo, name: check.name, head_sha: expectedSha, external_id: check.externalId,
    status: "completed", conclusion, output: { title, summary },
  };
  if (existing) {
    await github.rest.checks.update({ ...payload, check_run_id: existing.id });
  } else {
    await github.rest.checks.create(payload);
  }
  return true;
}

async function dispatchClassificationComplete(github, owner, repo, prNumber, expectedSha) {
  await assertCurrentHead(github, owner, repo, prNumber, expectedSha);
  await github.rest.repos.createDispatchEvent({
    owner, repo, event_type: "pr-automation-classified",
    client_payload: { pr_number: prNumber, head_sha: expectedSha },
  });
}

// Computes the whole label delta from a single read, so a state with two label sets plus additions
// and removals costs one issue read instead of one per candidate label.
async function applyLabels(github, owner, repo, prNumber, state, currentLabels) {
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
  await assertCurrentHead(github, owner, repo, prNumber, state.expectedSha);
  if (additions.length > 0) {
    await github.rest.issues.addLabels({ owner, repo, issue_number: prNumber, labels: additions });
  }
  for (const label of removals) {
    try {
      await github.rest.issues.removeLabel({ owner, repo, issue_number: prNumber, name: label });
    } catch (error) {
      if (error?.status !== 404) throw error;
    }
  }
  return true;
}

async function writeState({ github, owner, repo, prNumber, state, botLogin }) {
  if (!state?.ok || !["classification", "review"].includes(state.mode) ||
      typeof state.expectedSha !== "string" || !Number.isSafeInteger(prNumber) || prNumber <= 0) {
    throw new Error("invalid normalized state");
  }
  await assertCurrentHead(github, owner, repo, prNumber, state.expectedSha);
  if (state.mode === "review") {
    const comments = state.comments || [];
    for (const comment of comments.filter((comment) => comment.kind === "review")) {
      await publishReview(github, owner, repo, prNumber, state, botLogin, comment);
    }
    if (state.check) {
      await ensureReviewCheck(github, owner, repo, prNumber, state.expectedSha, state.check, state);
    }
    const latestLabels = state.failed === true
      ? undefined
      : await issueLabels(github, owner, repo, prNumber);
    if (latestLabels) assertReviewPolicy(latestLabels, state);
    await applyLabels(github, owner, repo, prNumber, state, latestLabels);
    for (const comment of comments.filter((comment) => comment.kind !== "review")) {
      await upsertMarkedComment(github, owner, repo, prNumber, state.expectedSha, botLogin, comment);
    }
    for (const marker of new Set(state.removeCommentMarkers || [])) {
      await deleteMarkedComment(github, owner, repo, prNumber, state.expectedSha, botLogin, marker);
    }
  } else {
    await applyLabels(github, owner, repo, prNumber, state);
    for (const comment of state.comments || []) await upsertMarkedComment(
      github, owner, repo, prNumber, state.expectedSha, botLogin, comment);
    for (const comment of state.auditComments || []) await upsertMarkedComment(
      github, owner, repo, prNumber, state.expectedSha, botLogin, comment);
    for (const marker of new Set(state.removeCommentMarkers || [])) {
      await deleteMarkedComment(github, owner, repo, prNumber, state.expectedSha, botLogin, marker);
    }
    if (state.check) {
      const created = await ensureClassificationCheck(github, owner, repo, prNumber, state.expectedSha, state.check);
      if (created && state.check.title === "Classification complete" && state.dispatchReview !== false) {
        await dispatchClassificationComplete(github, owner, repo, prNumber, state.expectedSha);
      }
    }
  }
  return { ok: true };
}

module.exports = {
  StaleHeadError, StalePolicyError, applyLabels, assertCurrentHead, deleteMarkedComment, dispatchClassificationComplete,
  escapeMarkdown, markerBody, upsertMarkedComment, writeState,
};
