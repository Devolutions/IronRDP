"use strict";

const { OVERSIZED_REVIEW_LABEL } = require("./resolve-state");

const SHA = /^[0-9a-f]{40}$/;

function noResult(reason, route = "unknown", pr = null, observedCiRun = null) {
  return {
    ok: false, route, reason,
    ...(observedCiRun ? { observedCiRun } : {}),
    ...(pr ? {
      prNumber: pr.number,
      headSha: pr.head?.sha,
      baseSha: pr.base?.sha,
      labels: (pr.labels || []).map((label) => typeof label === "string" ? label : label.name).filter(Boolean),
    } : {}),
  };
}

function routeFor(context) {
  return {
    pull_request_target: "classification",
    pull_request: "classification",
    workflow_run: "ci",
    repository_dispatch: "classification-complete",
    workflow_dispatch: "dispatch",
  }[context.eventName] ?? "unknown";
}

function positiveNumber(value) {
  const number = typeof value === "number" ? value : Number(value);
  return Number.isSafeInteger(number) && number > 0 ? number : null;
}

function inputFlag(value) {
  return ["true", true].includes(value);
}

async function getPull(github, owner, repo, number) {
  const { data: pr } = await github.rest.pulls.get({ owner, repo, pull_number: number });
  return SHA.test(pr.head?.sha || "") ? pr : null;
}

async function workflowRunPullRequests(github, owner, repo, workflowRun) {
  const wantedSha = workflowRun?.head_sha;
  if (!SHA.test(wantedSha || "")) return [];
  const candidates = new Set((workflowRun.pull_requests || []).map((pr) => positiveNumber(pr.number)).filter(Boolean));
  const matches = [];
  for (const number of candidates) {
    const pr = await getPull(github, owner, repo, number);
    if (pr?.head.sha === wantedSha) matches.push(pr);
  }
  if (matches.length > 0) return matches;

  for await (const response of github.paginate.iterator(github.rest.pulls.list, {
    owner, repo, state: "open", sort: "updated", direction: "desc", per_page: 100,
  })) {
    for (const candidate of response.data) {
      if (candidate.head?.sha !== wantedSha) continue;
      const pr = await getPull(github, owner, repo, candidate.number);
      if (pr?.head.sha === wantedSha) matches.push(pr);
    }
  }
  return matches;
}

async function resolvePr({ github, context, inputs = {} }) {
  const route = routeFor(context);
  const { owner, repo } = context.repo;
  const force = route === "dispatch" && inputFlag(inputs.force ?? context.payload.inputs?.force);
  const dispatchReview = route === "dispatch" &&
    inputFlag(inputs.review ?? context.payload.inputs?.review);
  const oversizedReviewRequested = route === "classification" &&
    context.payload.action === "labeled" && context.payload.label?.name === OVERSIZED_REVIEW_LABEL;
  let pr;
  let observedCiRun = null;
  try {
    if (route === "classification") {
      const number = positiveNumber(context.payload.pull_request?.number);
      if (!number) return noResult("missing pull request number", route);
      pr = await getPull(github, owner, repo, number);
    } else if (route === "dispatch") {
      const number = positiveNumber(inputs.prNumber ?? context.payload.inputs?.["pr-number"]);
      if (!number) return noResult("invalid dispatch pull request number", route);
      pr = await getPull(github, owner, repo, number);
    } else if (route === "ci") {
      const source = context.payload.workflow_run;
      const eventNumbers = new Set((source?.pull_requests || [])
        .map((candidate) => positiveNumber(candidate.number))
        .filter(Boolean));
      if (eventNumbers.size === 1) {
        pr = await getPull(github, owner, repo, [...eventNumbers][0]);
        if (pr && pr.head.sha !== source?.head_sha) {
          return noResult("workflow run head is stale", route, pr);
        }
      }
      if (!pr) {
        const matches = await workflowRunPullRequests(github, owner, repo, source);
        if (matches.length !== 1) return noResult("workflow run did not resolve exactly one current PR", route);
        pr = matches[0];
      }
      // The completed run is authoritative for its own generation while the run listing catches up.
      observedCiRun = {
        id: source.id, run_attempt: source.run_attempt, head_sha: source.head_sha,
        name: source.name, status: source.status, conclusion: source.conclusion,
      };
    } else if (route === "classification-complete") {
      if (context.payload.action !== "pr-automation-classified") {
        return noResult("unrelated repository dispatch", route);
      }
      const number = positiveNumber(context.payload.client_payload?.pr_number);
      const headSha = context.payload.client_payload?.head_sha;
      if (!number || !SHA.test(headSha || "")) return noResult("invalid classification dispatch", route);
      pr = await getPull(github, owner, repo, number);
      if (pr && pr.head.sha !== headSha) return noResult("classification dispatch head is stale", route, pr);
    } else {
      return noResult("unsupported event", route);
    }
  } catch {
    return noResult("GitHub API unavailable", route);
  }
  if (!pr) return noResult("pull request is stale", route);
  if (pr.state !== "open") return noResult("pull request is closed", route, pr, observedCiRun);
  if (pr.draft && !force) return noResult("pull request is draft", route, pr, observedCiRun);
  // Dependabot owns dependency and language labels, while devolutionsbot opens release-plz PRs.
  // This automation must not mutate either kind of automated pull request.
  const authorLogin = pr.user?.login || "";
  const authorIsBot = pr.user?.type === "Bot" || /\[bot\]$/i.test(authorLogin) ||
    authorLogin.toLowerCase() === "devolutionsbot";
  if (authorIsBot && !force) {
    return noResult("bot-authored pull request", route, pr, observedCiRun);
  }
  const labels = (pr.labels || [])
    .map((label) => typeof label === "string" ? label : label.name)
    .filter(Boolean);
  const unrelatedLabel = route === "classification" && ["labeled", "unlabeled"].includes(context.payload.action) &&
    context.payload.label?.name !== OVERSIZED_REVIEW_LABEL;
  if (unrelatedLabel) return noResult("unrelated pull request label", route, pr);
  const reviewRequested = oversizedReviewRequested || dispatchReview;
  const classificationRequested = route === "classification" ||
    (route === "dispatch" && !dispatchReview);
  return {
    ok: true, route, prNumber: pr.number, headSha: pr.head.sha, baseSha: pr.base.sha,
    labels,
    evidenceMaxBytes: labels.includes(OVERSIZED_REVIEW_LABEL) ? 4 * 1024 * 1024 : 1024 * 1024,
    author: {
      nodeId: pr.user?.node_id || null, login: pr.user?.login || null, type: pr.user?.type || null,
      association: pr.author_association || null,
    },
    force,
    observedCiRun,
    reviewRequested,
    classificationRequested,
    reviewRoute: route === "ci" || route === "classification-complete" || dispatchReview,
  };
}

module.exports = { positiveNumber, resolvePr, workflowRunPullRequests };
