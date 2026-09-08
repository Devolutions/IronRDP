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

// The caller owns which specialists are required. `required-reviewers` is authoritative when the
// caller sends it; the gate-derived route stays as the fallback so an older caller keeps working.
function resolveRequiredReviewers({
  selectedReviewers, requiredReviewers, protocolRelated, risk,
} = {}) {
  const selected = normalizeReviewerIds(selectedReviewers);
  if (!selected) return invalid("invalid specialist execution plan");
  if (Array.isArray(requiredReviewers) && requiredReviewers.length > 0) {
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

// Every provider failure the runtime reports is either worth another attempt later or is settled.
// Exhausted output repair is settled: the runtime already corrected inside the same conversation.
const RETRYABLE_CATEGORIES = new Set([
  "provider-timeout", "provider-connection", "provider-unavailable", "provider-transient",
]);

const REUSE_RECORD_VERSION = 1;

function isRetryableFailure(category) {
  return RETRYABLE_CATEGORIES.has(category);
}

function normalizeMetrics(metrics = {}) {
  const count = (value) => Number.isSafeInteger(value) && value >= 0 ? value : null;
  return {
    tokens: normalizeTokenUsage(metrics.tokens),
    elapsed_ms: count(metrics.elapsed_ms),
    request_retries: count(metrics.request_retries),
    output_repairs: count(metrics.output_repairs),
    stage_recoveries: count(metrics.stage_recoveries) ?? 0,
  };
}

// Providers report usage under several spellings, and the runtime marks whether every attempt was
// accounted for. A stage whose usage is partial must never look like a complete measurement.
function normalizeTokenUsage(tokens) {
  if (tokens === null || typeof tokens !== "object" || Array.isArray(tokens)) return null;
  const pick = (...keys) => {
    for (const key of keys) {
      const value = tokens[key];
      if (Number.isSafeInteger(value) && value >= 0) return value;
    }
    return null;
  };
  const input = pick("inputTokens", "input", "input_tokens", "prompt_tokens");
  const output = pick("outputTokens", "output", "output_tokens", "completion_tokens");
  const total = pick("totalTokens", "total", "total_tokens");
  if (input === null && output === null && total === null) return null;
  return {
    input,
    output,
    total: total ?? (input === null || output === null ? null : input + output),
    complete: tokens.complete === true && input !== null && output !== null && total !== null,
  };
}

function stageRecord({
  id, status, required = false, reason = "", failureCategory = "", retryable = false,
  reused = false, reusedFromRunId = null, reuseReason = "", provider = false, metrics = {},
} = {}) {
  return {
    id,
    status,
    required: required === true,
    provider: provider === true,
    reason: normalizeText(reason, 300) || "",
    failure_category: normalizeText(failureCategory, 60) || "",
    retryable: status === "failed" && retryable === true,
    reused: reused === true,
    reused_from_run_id: reused === true ? reusedFromRunId : null,
    // A discarded or missing cached result must be visible, never a silent rerun.
    reuse_reason: normalizeText(reuseReason, 200) || "",
    metrics: reused === true
      ? { ...normalizeMetrics(metrics), tokens: null }
      : normalizeMetrics(metrics),
  };
}

function summarizeStages(stages) {
  const totals = {
    tokens: { input: 0, output: 0, total: 0 },
    tokens_complete: true,
    elapsed_ms: 0,
    request_retries: 0,
    output_repairs: 0,
    stage_recoveries: 0,
    reused_stages: 0,
    failed_stages: 0,
  };
  for (const stage of stages) {
    const metrics = normalizeMetrics(stage.metrics ?? {});
    if (stage.reused) totals.reused_stages += 1;
    if (stage.status === "failed") totals.failed_stages += 1;
    if (metrics.tokens) {
      totals.tokens.input += metrics.tokens.input ?? 0;
      totals.tokens.output += metrics.tokens.output ?? 0;
      totals.tokens.total += metrics.tokens.total ?? 0;
      if (!metrics.tokens.complete) totals.tokens_complete = false;
    } else if (stage.provider && !stage.reused && stage.status !== "skipped") {
      // Reused work is known to cost nothing new, so it never makes the total incomplete.
      totals.tokens_complete = false;
    }
    for (const key of ["elapsed_ms", "request_retries", "output_repairs", "stage_recoveries"]) {
      if (metrics[key] !== null) totals[key] += metrics[key];
    }
  }
  const failed = stages.filter((stage) => stage.status === "failed");
  const published = stages.some((stage) => stage.id === "validate" && stage.status === "success");
  return {
    metrics: totals,
    // Dependency-skipped stages are not failures, and an optional terminal failure never blocks a
    // recovery that a required transient failure could still fix.
    recoverable: !published && failed.some((stage) => stage.retryable) &&
      !failed.some((stage) => stage.required && !stage.retryable),
    failures: failed.map(({ id, reason, failure_category: category, required, retryable }) =>
      ({ id, reason, failure_category: category, required, retryable })),
  };
}

function reusableRecord({ stage, stageKey, runId, attemptId, output, metrics }) {
  return {
    v: REUSE_RECORD_VERSION,
    stage,
    stage_key: stageKey,
    run_id: String(runId),
    attempt_id: attemptId,
    created_at: new Date().toISOString(),
    output,
    metrics: normalizeMetrics(metrics),
  };
}

// A restored result is only a candidate. It still has to prove it belongs to this exact review, and
// it is revalidated by the current rules before anything downstream may depend on it.
function acceptReusableRecord(record, { stage, expectedStageKey } = {}) {
  if (record === null || typeof record !== "object" || Array.isArray(record)) {
    return { ok: false, reason: "cached result is unreadable" };
  }
  if (record.v !== REUSE_RECORD_VERSION) {
    return { ok: false, reason: "cached result uses an unsupported record version" };
  }
  if (record.stage !== stage) {
    return { ok: false, reason: "cached result belongs to a different stage" };
  }
  if (typeof record.stage_key !== "string" || record.stage_key !== expectedStageKey) {
    return { ok: false, reason: "cached result does not match the current review inputs" };
  }
  if (typeof record.output !== "string" || record.output === "") {
    return { ok: false, reason: "cached result carries no model output" };
  }
  return { ok: true, value: record };
}

module.exports = {
  RETRYABLE_CATEGORIES, REUSE_RECORD_VERSION, SPECIALIST_ORDER,
  acceptReusableRecord, buildSpecialistAggregate, failedRun, isRetryableFailure, normalizeMetrics,
  resolveRequiredReviewers, reusableRecord, stageRecord, summarizeStages, validateSpecialistRun,
};
