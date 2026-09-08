"use strict";

// One canonical report travels from the review pipeline to its caller. Both sides normalize it
// here so the producer and the consumer can never drift into two slightly different schemas.

const { normalizeText } = require("./validation");

const REPORT_VERSION = 1;
const STAGE_STATUS = new Set(["success", "failed", "skipped"]);

function count(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

// Providers report usage under several spellings, and the runtime marks whether every attempt was
// accounted for. A stage whose usage is partial must never look like a complete measurement.
function normalizeTokens(tokens) {
  if (tokens === null || typeof tokens !== "object" || Array.isArray(tokens)) return null;
  const pick = (...keys) => {
    for (const key of keys) {
      const value = count(tokens[key]);
      if (value !== null) return value;
    }
    return null;
  };
  const input = pick("input", "inputTokens", "input_tokens", "prompt_tokens");
  const output = pick("output", "outputTokens", "output_tokens", "completion_tokens");
  const total = pick("total", "totalTokens", "total_tokens");
  if (input === null && output === null && total === null) return null;
  return {
    input,
    output,
    total: total ?? (input === null || output === null ? null : input + output),
    complete: tokens.complete === true && input !== null && output !== null,
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

// `attempts` is 1 or 2 because a stage gets at most one delayed retry. `previous_reason` keeps the
// first attempt's failure visible even when the retry succeeded, so a recovered stage still
// explains what went wrong.
function stageOutcome({
  id, status, required = false, reason = "", category = "", attempts = 1, previousReason = "",
  previous_reason: previousReasonKey = "", provider = false, metrics = {},
} = {}) {
  return {
    id: String(id ?? ""),
    status: STAGE_STATUS.has(status) ? status : "failed",
    required: required === true,
    provider: provider === true,
    reason: normalizeText(reason, 300) || "",
    category: normalizeText(category, 60) || "",
    attempts: attempts === 2 ? 2 : 1,
    previous_reason: normalizeText(previousReason || previousReasonKey, 300) || "",
    metrics: normalizeStageMetrics(metrics),
  };
}

function buildReport(stages = []) {
  const outcomes = (Array.isArray(stages) ? stages : []).map(stageOutcome);
  const metrics = {
    tokens: { input: 0, output: 0, total: 0 },
    tokens_complete: true,
    elapsed_ms: 0,
    request_retries: 0,
    output_repairs: 0,
    stage_retries: 0,
  };
  for (const stage of outcomes) {
    if (stage.attempts === 2) metrics.stage_retries += 1;
    if (stage.metrics.tokens) {
      metrics.tokens.input += stage.metrics.tokens.input ?? 0;
      metrics.tokens.output += stage.metrics.tokens.output ?? 0;
      metrics.tokens.total += stage.metrics.tokens.total ?? 0;
      if (!stage.metrics.tokens.complete) metrics.tokens_complete = false;
    } else if (stage.provider && stage.status !== "skipped") {
      metrics.tokens_complete = false;
    }
    for (const key of ["elapsed_ms", "request_retries", "output_repairs"]) {
      if (stage.metrics[key] !== null) metrics[key] += stage.metrics[key];
    }
  }
  const published = outcomes.some((stage) =>
    stage.id === "validate" && stage.status === "success");
  const failedRequired = outcomes.some((stage) =>
    stage.required && stage.status === "failed");
  return {
    v: REPORT_VERSION,
    status: published && !failedRequired ? "success" : "failed",
    stages: outcomes.map(({ provider, ...stage }) => stage),
    metrics,
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
    metrics: buildReport([]).metrics,
  });
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return unusable("the review pipeline returned no usable report");
  }
  if (parsed.v !== REPORT_VERSION) {
    return unusable("the review pipeline returned an unsupported report version");
  }
  const report = buildReport(parsed.stages);
  // The `provider` flag does not survive the wire, so a stage whose usage was never measured looks
  // measurable to the consumer. Honour the producer's own completeness flag, but only downwards:
  // a producer can tell us it knows less than we inferred, never more.
  if (parsed.metrics?.tokens_complete === false) report.metrics.tokens_complete = false;
  // A producer that says it failed is believed; a producer that says it succeeded still has to
  // satisfy the same published-and-no-required-failure rule the pipeline applies.
  return parsed.status === "failed" ? { ...report, status: "failed" } : report;
}

function stageIds(report) {
  return (report?.stages ?? []).map((stage) => stage.id);
}

module.exports = {
  REPORT_VERSION, buildReport, normalizeStageMetrics, parseReport, stageIds, stageOutcome,
};
