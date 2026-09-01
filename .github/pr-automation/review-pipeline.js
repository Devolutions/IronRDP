"use strict";

const { normalizeCandidateReview } = require("./validate-candidate-review");
const { validateProtocolReferences } = require("./validate-protocol-review");
const {
  REVIEWER_ORDER: SPECIALIST_ORDER, normalizeReviewerIds, resolveReviewerRoute,
} = require("./routing");
const { SHA, exactKeys, invalid, normalizeText } = require("./validation");

function validateSpecialistRun(raw, {
  reviewer, expectedSha, changedPaths, changedLines, corpus, expectedCorpusSha, failureReason,
} = {}) {
  if (!SPECIALIST_ORDER.includes(reviewer) || !SHA.test(expectedSha || "")) {
    return invalid("invalid specialist validation context");
  }
  const result = normalizeCandidateReview(raw, {
    expectedSha,
    expectedReviewer: reviewer,
    changedPaths,
    changedLines,
  });
  if (!result.ok) return failedRun(reviewer, failureReason || result.reason);

  if (reviewer === "protocol") {
    if (!corpus?.isPinnedTo?.(expectedCorpusSha)) {
      return failedRun(reviewer, "protocol corpus commit mismatch");
    }
    for (const finding of result.value.findings) {
      const references = validateProtocolReferences(finding.references, {
        corpus,
        expectedCorpusSha,
      });
      if (!references.ok) return failedRun(reviewer, references.reason);
      finding.references = references.value;
    }
  }

  return {
    ok: true,
    value: {
      reviewer,
      status: "valid",
      summary: result.value.summary,
      findings: result.value.findings,
    },
  };
}

function failedRun(reviewer, reason) {
  const normalizedReason = normalizeText(reason, 300) || "specialist unavailable";
  return {
    ok: false,
    reason: normalizedReason,
    value: { reviewer, status: "failed", reason: normalizedReason },
  };
}

function buildSpecialistAggregate({
  expectedSha, selectedReviewers, runs, protocolRelated, risk,
} = {}) {
  if (!SHA.test(expectedSha || "")) return invalid("invalid specialist aggregate SHA");
  const selected = normalizeReviewerIds(selectedReviewers);
  if (!selected || selected.some((reviewer, index) => reviewer !== selectedReviewers[index])) {
    return invalid("invalid specialist execution plan");
  }
  if (!Array.isArray(runs) || runs.length !== selected.length) {
    return invalid("incomplete specialist execution");
  }

  const reviewers = [];
  for (const [index, reviewer] of selected.entries()) {
    const run = runs[index];
    if (!run || run.reviewer !== reviewer ||
        (run.status === "valid"
          ? !exactKeys(run, ["reviewer", "status", "summary", "findings"]) ||
            !Array.isArray(run.findings)
          : run.status !== "failed" || !exactKeys(run, ["reviewer", "status", "reason"]))) {
      return invalid("invalid specialist execution result");
    }
    reviewers.push(run);
  }

  const aggregate = { head_sha: expectedSha, reviewers };
  if (Buffer.byteLength(JSON.stringify(aggregate), "utf8") > 128 * 1024) {
    return invalid("specialist aggregate too large");
  }
  const mandatory = resolveReviewerRoute({
    suggestedReviewers: [], protocolRelated, risk,
  });
  if (!mandatory.ok) return invalid("invalid mandatory reviewer policy");
  const failedMandatory = reviewers.find((reviewer) =>
    reviewer.status === "failed" && mandatory.reviewers.includes(reviewer.reviewer));
  return {
    ok: true,
    value: aggregate,
    mandatoryFailure: failedMandatory?.reason || "",
  };
}

module.exports = {
  SPECIALIST_ORDER, buildSpecialistAggregate, failedRun, validateSpecialistRun,
};
