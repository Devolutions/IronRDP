"use strict";

function generation(run) {
  const id = Number(run?.id);
  const attempt = Number(run?.run_attempt);
  return Number.isSafeInteger(id) && id > 0 && Number.isSafeInteger(attempt) && attempt > 0
    ? { id, attempt }
    : null;
}

function latestExactHeadCiRun(runs, expectedSha) {
  return (runs || [])
    .filter((run) => run?.name === "CI" && run?.head_sha === expectedSha && generation(run))
    .sort((left, right) =>
      Number(right.id) - Number(left.id) ||
      Number(right.run_attempt) - Number(left.run_attempt))[0] ?? null;
}

async function readLatestExactHeadCiRun({ github, owner, repo, expectedSha }) {
  const runs = [];
  for await (const response of github.paginate.iterator(github.rest.actions.listWorkflowRunsForRepo, {
    owner, repo, head_sha: expectedSha, per_page: 100,
  })) {
    // Octokit's paginator normalizes this endpoint to its `workflow_runs` array.
    // Keep the namespaced form for narrow test doubles and older wrappers.
    const page = Array.isArray(response.data) ? response.data : response.data?.workflow_runs;
    if (Array.isArray(page)) runs.push(...page);
  }
  return latestExactHeadCiRun(runs, expectedSha);
}

function matchesGeneration(run, id, attempt) {
  const current = generation(run);
  return current?.id === Number(id) && current.attempt === Number(attempt);
}

module.exports = { generation, latestExactHeadCiRun, matchesGeneration, readLatestExactHeadCiRun };
