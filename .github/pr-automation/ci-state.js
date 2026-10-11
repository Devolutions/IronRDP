"use strict";

function generation(run) {
  const id = Number(run?.id);
  const attempt = Number(run?.run_attempt);
  return Number.isSafeInteger(id) && id > 0 && Number.isSafeInteger(attempt) && attempt > 0
    ? { id, attempt }
    : null;
}

function compareGenerations(left, right) {
  return left.id - right.id || left.attempt - right.attempt;
}

function exactHeadCiGeneration(run, expectedSha) {
  return run?.name === "CI" && run?.head_sha === expectedSha ? generation(run) : null;
}

function statusRank(status) {
  switch (status) {
    case "completed": return 3;
    case "in_progress": return 2;
    // `requested` is a workflow_run event action; `queued` is the corresponding run status.
    case "queued":
    case "requested": return 1;
    default: return 0;
  }
}

// The triggering `workflow_run` event can be ahead of the listing. For the same generation,
// retain whichever source reports the more advanced GitHub workflow status. A newer generation
// in the listing still wins regardless of status.
function latestExactHeadCiRun(runs, expectedSha, observedRun = null) {
  const observed = exactHeadCiGeneration(observedRun, expectedSha);
  const candidates = (runs || []).filter((run) => exactHeadCiGeneration(run, expectedSha));
  if (observed) candidates.push(observedRun);
  return candidates.sort((left, right) =>
    compareGenerations(generation(right), generation(left)) ||
    statusRank(right.status) - statusRank(left.status))[0] ?? null;
}

// Re-lists only while the listing is behind: no candidate, a candidate older than the expected
// generation, a candidate that has not completed, or, without a reference generation, one that did
// not succeed. A newer generation than the reference is returned at once.
async function readLatestExactHeadCiRun({
  github, owner, repo, expectedSha, observedRun = null, expectedGeneration = null,
  retries = 3, delayMs = 3000, sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  const reference = generation({ id: expectedGeneration?.id, run_attempt: expectedGeneration?.attempt }) ??
    exactHeadCiGeneration(observedRun, expectedSha);
  const behind = (run) => {
    const current = generation(run);
    if (!current) return true;
    const order = reference ? compareGenerations(current, reference) : 0;
    if (order !== 0) return order < 0;
    // A conclusion exists only once a run completes; accept it for payloads without `status`.
    if (!(run.status === "completed" || (run.status === undefined && run.conclusion != null))) return true;
    // Without a reference generation, an older unsuccessful run may still mask a newer one the
    // listing has not caught up to.
    return !reference && run.conclusion !== "success";
  };
  for (let attempt = 0; ; attempt += 1) {
    const runs = [];
    for await (const response of github.paginate.iterator(github.rest.actions.listWorkflowRunsForRepo, {
      owner, repo, head_sha: expectedSha, per_page: 100,
    })) {
      // Octokit's paginator normalizes this endpoint to its `workflow_runs` array.
      // Keep the namespaced form for narrow test doubles and older wrappers.
      const page = Array.isArray(response.data) ? response.data : response.data?.workflow_runs;
      if (Array.isArray(page)) runs.push(...page);
    }
    const latest = latestExactHeadCiRun(runs, expectedSha, observedRun);
    if (attempt >= retries || !behind(latest)) return latest;
    if (delayMs > 0) await sleep(delayMs);
  }
}

function matchesGeneration(run, id, attempt) {
  const current = generation(run);
  return current?.id === Number(id) && current.attempt === Number(attempt);
}

module.exports = { generation, latestExactHeadCiRun, matchesGeneration, readLatestExactHeadCiRun };
