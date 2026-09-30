"use strict";

async function readCheckRuns({ github, owner, repo, ref, checkName }) {
  const runs = [];
  for await (const response of github.paginate.iterator(github.rest.checks.listForRef, {
    owner, repo, ref, check_name: checkName, per_page: 100,
  })) {
    if (Array.isArray(response.data)) runs.push(...response.data);
  }
  return runs;
}

module.exports = { readCheckRuns };
