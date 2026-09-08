"use strict";

const { SCHEMA_VERSION, parseCheckState } = require("./validate-classifier");

async function resolveClassificationGate({
  github, owner, repo, expectedSha, force = false, retryWithLargerEvidence = false,
}) {
  if (force) {
    return { available: true, required: true, reason: "", force: true };
  }
  if (retryWithLargerEvidence) {
    return { available: true, required: true, reason: "", largerEvidence: true };
  }
  try {
    const { data } = await github.rest.checks.listForRef({
      owner, repo, ref: expectedSha, check_name: "AI classification", per_page: 100,
    });
    const externalId = `${SCHEMA_VERSION}:${expectedSha}`;
    const completed = data.check_runs.some((run) => {
      const state = parseCheckState(run.output?.summary);
      return run.external_id === externalId && run.conclusion === "success" &&
        run.app?.slug === "github-actions" && state?.automaticReviewEligible === true &&
        run.output?.title === "Classification complete";
    });
    return { available: true, required: !completed, reason: "", externalId, completed };
  } catch (error) {
    return {
      available: false,
      required: false,
      reason: "GitHub checks API unavailable",
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

module.exports = { resolveClassificationGate };
