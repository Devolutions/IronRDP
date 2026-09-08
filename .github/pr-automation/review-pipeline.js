"use strict";

const { normalizeCandidateReview } = require("./validate-candidate-review");
const { validateProtocolReferences } = require("./validate-protocol-review");
const {
  REVIEWER_ORDER: SPECIALIST_ORDER, normalizeReviewerIds, resolveReviewerRoute,
} = require("./routing");
const { SHA, exactKeys, invalid, normalizeText } = require("./validation");
const { normalizeStageMetrics } = require("./review-report");

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

// The caller owns which specialists are required. `required-reviewers` is authoritative when the
// caller sends it; the gate-derived route stays as the fallback so an older caller keeps working.
function resolveRequiredReviewers({
  selectedReviewers, requiredReviewers, protocolRelated, risk,
} = {}) {
  const selected = normalizeReviewerIds(selectedReviewers);
  if (!selected) return invalid("invalid specialist execution plan");
  if (requiredReviewers != null) {
    if (!Array.isArray(requiredReviewers)) return invalid("invalid required reviewer list");
    if (requiredReviewers.length === 0) return { ok: true, reviewers: [], source: "caller" };
    const required = normalizeReviewerIds(requiredReviewers);
    if (!required || required.some((reviewer, index) => reviewer !== requiredReviewers[index])) {
      return invalid("invalid required reviewer list");
    }
    if (required.some((reviewer) => !selected.includes(reviewer))) {
      return invalid("required reviewer was not selected");
    }
    return { ok: true, reviewers: required, source: "caller" };
  }
  const route = resolveReviewerRoute({ suggestedReviewers: [], protocolRelated, risk });
  if (!route.ok) return invalid("invalid mandatory reviewer policy");
  if (route.reviewers.some((reviewer) => !selected.includes(reviewer))) {
    return invalid("required reviewer was not selected");
  }
  return { ok: true, reviewers: route.reviewers, source: "gate" };
}

function buildSpecialistAggregate({
  expectedSha, selectedReviewers, runs, requiredReviewers, protocolRelated, risk,
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
  const mandatory = resolveRequiredReviewers({
    selectedReviewers: selected, requiredReviewers, protocolRelated, risk,
  });
  if (!mandatory.ok) return invalid(mandatory.reason);
  const failedMandatory = reviewers.filter((reviewer) =>
    reviewer.status === "failed" && mandatory.reviewers.includes(reviewer.reviewer));
  return {
    ok: true,
    value: aggregate,
    requiredReviewers: mandatory.reviewers,
    // Report every failed required specialist, not just the first one.
    mandatoryFailure: failedMandatory
      .map(({ reviewer, reason }) => `${reviewer}: ${reason}`)
      .join("; "),
  };
}

// The runtime classifies its own failures and reports `retryable`. Re-deriving that here from
// category names would silently diverge from it.
function isRetryableFailure(retryable) {
  return retryable === true || retryable === "true";
}

// The runtime reports measurements in one canonical diagnostics object, freshly built per
// invocation. A stage that cannot read it reports every measurement as unavailable, never as zero.
function parseDiagnostics(raw) {
  const parsed = (() => {
    if (raw === null || raw === undefined || raw === "") return null;
    if (typeof raw !== "string") return raw;
    try {
      return JSON.parse(raw);
    } catch {
      return null;
    }
  })();
  const source = parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)
    ? parsed
    : {};
  const count = (value) => Number.isSafeInteger(value) && value >= 0 ? value : null;
  const usage = source.tokenUsage !== null && typeof source.tokenUsage === "object"
    ? source.tokenUsage
    : {};
  return {
    elapsed_ms: count(source.durationMs),
    request_retries: count(source.requestRetryCount),
    output_repairs: count(source.outputRepairCount),
    provider_attempts: Array.isArray(source.providerAttempts) ? source.providerAttempts.length : null,
    tokens: normalizeStageMetrics({
      tokens: {
        input: usage.inputTokens,
        output: usage.outputTokens,
        total: usage.totalTokens,
        complete: usage.complete === true,
      },
    }).tokens,
  };
}

// Diagnostics are per invocation, so a retried stage spent both attempts. Summing keeps the cost
// honest, and one unmeasured attempt must not silently disappear into the other's number.
function mergeDiagnostics(first, second) {
  if (!second) return first;
  if (!first) return second;
  const add = (left, right) => left === null || right === null ? null : left + right;
  const tokens = (() => {
    if (!first.tokens && !second.tokens) return null;
    if (!first.tokens || !second.tokens) return { ...(first.tokens ?? second.tokens), complete: false };
    return {
      input: add(first.tokens.input, second.tokens.input),
      output: add(first.tokens.output, second.tokens.output),
      total: add(first.tokens.total, second.tokens.total),
      complete: first.tokens.complete && second.tokens.complete,
    };
  })();
  return {
    elapsed_ms: add(first.elapsed_ms, second.elapsed_ms),
    request_retries: add(first.request_retries, second.request_retries),
    output_repairs: add(first.output_repairs, second.output_repairs),
    provider_attempts: add(first.provider_attempts, second.provider_attempts),
    tokens,
  };
}

// A stage that never reached the provider spent nothing, so its zero is a measurement rather than a
// gap in the report. Diagnostics that cannot be read prove nothing, so they still count as spending.
function providerWasCalled(diagnostics) {
  const attempts = diagnostics?.provider_attempts;
  return attempts === null || attempts === undefined || attempts > 0;
}

// The mandatory set is resolved once, in the evidence job. A stage that cannot read that plan
// treats every selected reviewer as mandatory rather than none.
function plannedRequiredReviewers(raw, selectedReviewers = []) {
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : selectedReviewers;
  } catch {
    return selectedReviewers;
  }
}

module.exports = {
  SPECIALIST_ORDER,
  buildSpecialistAggregate, failedRun, isRetryableFailure, mergeDiagnostics, parseDiagnostics,
  plannedRequiredReviewers, providerWasCalled, resolveRequiredReviewers, validateSpecialistRun,
};
