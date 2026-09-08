"use strict";

// One canonical report travels from the review pipeline to its caller. Both sides normalize it
// here so the producer and the consumer can never drift into two slightly different schemas.

const { normalizeText } = require("./validation");

const REPORT_VERSION = 1;
const STAGE_STATUS = new Set(["success", "failed", "skipped"]);

function count(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

// Usage arrives in the report spelling, carrying whether every attempt was accounted for. A stage
// whose usage is partial must never look like a complete measurement.
function normalizeTokens(tokens) {
  if (tokens === null || typeof tokens !== "object" || Array.isArray(tokens)) return null;
  const input = count(tokens.input);
  const output = count(tokens.output);
  const total = count(tokens.total);
  if (input === null && output === null && total === null) return null;
  // Usage the producer did not report stays unreported. Deriving a total from two of three fields
  // would turn a partial measurement into one that looks whole.
  return {
    input,
    output,
    total,
    complete: tokens.complete === true && input !== null && output !== null && total !== null,
  };
}

function normalizeStageMetrics(metrics = {}) {
  const source = metrics === null || typeof metrics !== "object" ? {} : metrics;
  return {
    tokens: normalizeTokens(source.tokens),
    elapsed_ms: count(source.elapsed_ms),
    request_retries: count(source.request_retries),
    output_repairs: count(source.output_repairs),
  };
}

const MANDATORY_STAGES = ["evidence", "aggregate", "general", "validate"];

// `attempts` is 1 or 2 because a stage gets at most one delayed retry. `previous_reason` keeps the
// first attempt's failure visible even when the retry succeeded.
function stageOutcome(raw) {
  const {
    id, status, required = false, reason = "", category = "", attempts = 1,
    previous_reason: previousReason = "", provider = false, metrics = {},
  } = raw !== null && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
  return {
    id: typeof id === "string" ? id : "",
    status: STAGE_STATUS.has(status) ? status : "failed",
    required: required === true,
    provider: provider === true,
    reason: normalizeText(reason, 300) || "",
    category: normalizeText(category, 60) || "",
    attempts: attempts === 2 ? 2 : 1,
    previous_reason: normalizeText(previousReason, 300) || "",
    metrics: normalizeStageMetrics(metrics),
  };
}

const UNKNOWN_METRICS = {
  tokens: null,
  tokens_complete: false,
  elapsed_ms: null,
  request_retries: null,
  output_repairs: null,
  stage_retries: null,
};

function aggregateMetrics(outcomes) {
  const metrics = {
    tokens: { input: 0, output: 0, total: 0 },
    tokens_complete: true,
    elapsed_ms: 0,
    request_retries: 0,
    output_repairs: 0,
    stage_retries: 0,
  };
  let anyTokens = false;
  for (const stage of outcomes) {
    if (stage.attempts === 2) metrics.stage_retries += 1;
    const ran = stage.status !== "skipped";
    if (stage.metrics.tokens) {
      anyTokens = true;
      for (const key of ["input", "output", "total"]) {
        if (stage.metrics.tokens[key] === null) metrics.tokens[key] = null;
        else if (metrics.tokens[key] !== null) metrics.tokens[key] += stage.metrics.tokens[key];
      }
      if (!stage.metrics.tokens.complete) metrics.tokens_complete = false;
    } else if (stage.provider && ran) {
      metrics.tokens_complete = false;
    }
    // A stage that ran without reporting a measurement makes the total unknown, never a smaller
    // number that reads as measured. Retries and repairs are provider counters, so only a provider
    // stage can leave them unknown.
    for (const key of ["elapsed_ms", "request_retries", "output_repairs"]) {
      const measurable = ran && (key === "elapsed_ms" || stage.provider);
      if (stage.metrics[key] === null) {
        if (measurable) metrics[key] = null;
      } else if (metrics[key] !== null) {
        metrics[key] += stage.metrics[key];
      }
    }
  }
  if (!anyTokens) metrics.tokens = null;
  return metrics;
}

// A report is successful only when its shape proves it: every mandatory stage present exactly once
// and no required stage left unfinished, with an independent validation that actually succeeded.
function buildReport(stages = []) {
  const outcomes = (Array.isArray(stages) ? stages : []).map(stageOutcome);
  const ids = outcomes.map((stage) => stage.id);
  const wellFormed = ids.every((id) => id !== "") &&
    new Set(ids).size === ids.length &&
    MANDATORY_STAGES.every((id) => ids.includes(id));
  const published = outcomes.some((stage) =>
    stage.id === "validate" && stage.status === "success");
  const requiredUnfinished = outcomes.some((stage) =>
    stage.required && stage.status !== "success");
  return {
    v: REPORT_VERSION,
    status: wellFormed && published && !requiredUnfinished ? "success" : "failed",
    stages: outcomes,
    metrics: aggregateMetrics(outcomes),
  };
}

// The caller must never crash on a report and must never read a malformed one as success.
function parseReport(raw) {
  const parsed = (() => {
    if (raw === null || raw === undefined || raw === "") return null;
    if (typeof raw !== "string") return raw;
    try {
      return JSON.parse(raw);
    } catch {
      return null;
    }
  })();
  const unusable = (reason) => ({
    v: REPORT_VERSION,
    status: "failed",
    stages: [stageOutcome({ id: "pipeline", status: "failed", required: true, reason })],
    metrics: { ...UNKNOWN_METRICS },
  });
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return unusable("the review pipeline returned no usable report");
  }
  if (parsed.v !== REPORT_VERSION) {
    return unusable("the review pipeline returned an unsupported report version");
  }
  if (!Array.isArray(parsed.stages) || parsed.stages.length === 0) {
    return unusable("the review pipeline reported no stages");
  }
  const report = buildReport(parsed.stages);
  // The producer's own completeness flag is honoured downwards: a report may know less than the
  // stages suggest, never more.
  if (parsed.metrics?.tokens_complete === false) report.metrics.tokens_complete = false;
  return parsed.status === "failed" ? { ...report, status: "failed" } : report;
}

function stageIds(report) {
  return (report?.stages ?? []).map((stage) => stage.id);
}

module.exports = {
  MANDATORY_STAGES, REPORT_VERSION,
  buildReport, normalizeStageMetrics, parseReport, stageIds, stageOutcome,
};
