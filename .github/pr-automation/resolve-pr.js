"use strict";

const { OVERSIZED_REVIEW_LABEL } = require("./resolve-state");

const SHA = /^[0-9a-f]{40}$/;

function noResult(reason, route = "unknown") {
  return { ok: false, route, reason };
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

async function getOpenAtHead(github, owner, repo, number, requiredSha) {
  const { data: pr } = await github.rest.pulls.get({ owner, repo, pull_number: number });
  return pr.state === "open" && SHA.test(pr.head?.sha || "") && (!requiredSha || pr.head.sha === requiredSha) ? pr : null;
}

async function workflowRunPullRequests(github, owner, repo, workflowRun) {
  const wantedSha = workflowRun?.head_sha;
  if (!SHA.test(wantedSha || "")) return [];
  const candidates = new Set((workflowRun.pull_requests || []).map((pr) => positiveNumber(pr.number)).filter(Boolean));
  const matches = [];
  for (const number of candidates) {
    const pr = await getOpenAtHead(github, owner, repo, number, wantedSha);
    if (pr) matches.push(pr);
  }
  if (matches.length > 0) return matches;

  for await (const response of github.paginate.iterator(github.rest.pulls.list, {
    owner, repo, state: "open", sort: "updated", direction: "desc", per_page: 100,
  })) {
    for (const candidate of response.data) {
      if (candidate.head?.sha !== wantedSha) continue;
      const pr = await getOpenAtHead(github, owner, repo, candidate.number, wantedSha);
      if (pr) matches.push(pr);
    }
  }
  return matches;
}

async function resolvePr({ github, context, inputs = {} }) {
  const route = routeFor(context);
  const { owner, repo } = context.repo;
  const force = route === "dispatch" && ["true", true].includes(
    inputs.force ?? context.payload.inputs?.force);
  let pr;
  try {
    if (route === "classification") {
      // State writes also emit `labeled` events, so only the explicit maintainer opt-in may start
      // automation through that event.
      if (context.payload.action === "labeled" && context.payload.label?.name !== OVERSIZED_REVIEW_LABEL) {
        return noResult("unrelated pull request label", route);
      }
      const number = positiveNumber(context.payload.pull_request?.number);
      if (!number) return noResult("missing pull request number", route);
      pr = await getOpenAtHead(github, owner, repo, number);
    } else if (route === "dispatch") {
      const number = positiveNumber(inputs.prNumber ?? context.payload.inputs?.["pr-number"]);
      if (!number) return noResult("invalid dispatch pull request number", route);
      pr = await getOpenAtHead(github, owner, repo, number);
    } else if (route === "ci") {
      const source = context.payload.workflow_run;
      const matches = await workflowRunPullRequests(github, owner, repo, source);
      if (matches.length !== 1) return noResult("workflow run did not resolve exactly one current PR", route);
      pr = matches[0];
    } else if (route === "classification-complete") {
      if (context.payload.action !== "pr-automation-classified") {
        return noResult("unrelated repository dispatch", route);
      }
      const number = positiveNumber(context.payload.client_payload?.pr_number);
      const headSha = context.payload.client_payload?.head_sha;
      if (!number || !SHA.test(headSha || "")) return noResult("invalid classification dispatch", route);
      pr = await getOpenAtHead(github, owner, repo, number, headSha);
    } else {
      return noResult("unsupported event", route);
    }
  } catch {
    return noResult("GitHub API unavailable", route);
  }
  if (!pr) return noResult("pull request is closed or stale", route);
  if (pr.draft && !force) return noResult("pull request is draft", route);
  // Dependabot owns dependency and language labels, while devolutionsbot opens release-plz PRs.
  // This automation must not mutate either kind of automated pull request.
  const authorLogin = pr.user?.login || "";
  const authorIsBot = pr.user?.type === "Bot" || /\[bot\]$/i.test(authorLogin) ||
    authorLogin.toLowerCase() === "devolutionsbot";
  if (authorIsBot && !force) return noResult("bot-authored pull request", route);
  return {
    ok: true, route, prNumber: pr.number, headSha: pr.head.sha, baseSha: pr.base.sha,
    labels: (pr.labels || []).map((label) => typeof label === "string" ? label : label.name).filter(Boolean),
    author: {
      nodeId: pr.user?.node_id || null, login: pr.user?.login || null, type: pr.user?.type || null,
      association: pr.author_association || null,
    },
    force,
    reviewRequested: route === "dispatch" && ["true", true].includes(
      inputs.review ?? context.payload.inputs?.review),
    classificationRequested: route === "classification" ||
      (route === "dispatch" && !["true", true].includes(inputs.review ?? context.payload.inputs?.review)),
    reviewRoute: route === "ci" || route === "classification-complete" ||
      (route === "dispatch" && ["true", true].includes(inputs.review ?? context.payload.inputs?.review)),
  };
}

module.exports = { positiveNumber, resolvePr, workflowRunPullRequests };
