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

// The triggering `workflow_run` event is authoritative for its own generation because the run
// listing can lag behind it. A newer generation in the listing still wins.
function latestExactHeadCiRun(runs, expectedSha, observedRun = null) {
  const observed = exactHeadCiGeneration(observedRun, expectedSha);
  const candidates = (runs || []).filter((run) => {
    const current = exactHeadCiGeneration(run, expectedSha);
    return current && !(observed && compareGenerations(current, observed) === 0);
  });
  if (observed) candidates.push(observedRun);
  return candidates.sort((left, right) => compareGenerations(generation(right), generation(left)))[0] ?? null;
}

// Re-lists only while the listing is behind: no candidate, a candidate older than the expected
// generation, or a candidate that has not completed. A newer generation is returned at once.
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
    return !(run.status === "completed" || (run.status === undefined && run.conclusion != null));
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
