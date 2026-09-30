"use strict";

const { SCHEMA_VERSION, parseCheckState } = require("./validate-classifier");
const { readCheckRuns } = require("./check-runs");

async function resolveClassificationGate({
  github, owner, repo, expectedSha, force = false, retryWithLargerEvidence = false,
}) {
  if (force) {
    return { available: true, required: true, reason: "", force: true };
  }
  try {
    const runs = await readCheckRuns({
      github, owner, repo, ref: expectedSha, checkName: "AI classification",
    });
    const externalId = `${SCHEMA_VERSION}:${expectedSha}`;
    const completed = runs.some((run) => {
      const state = parseCheckState(run.output?.summary);
      return run.external_id === externalId && run.conclusion === "success" &&
        run.app?.slug === "github-actions" && state?.automaticReviewEligible === true &&
        run.output?.title === "Classification complete";
    });
    return {
      available: true,
      required: retryWithLargerEvidence || !completed,
      reason: "",
      externalId,
      completed,
    };
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
