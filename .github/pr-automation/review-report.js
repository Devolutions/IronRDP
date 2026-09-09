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

// A stage that ran was attempted once, or twice when it took its single delayed retry, and a
// skipped stage was never attempted at all. `previous_reason` keeps the first attempt's failure
// visible even when the retry succeeded.
function stageOutcome(raw) {
  const {
    id, status, required = false, reason = "", category = "", attempts = 1,
    previous_reason: previousReason = "", provider = false, metrics = {},
  } = raw !== null && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
  const outcome = STAGE_STATUS.has(status) ? status : "failed";
  return {
    id: typeof id === "string" ? id : "",
    status: outcome,
    required: required === true,
    provider: provider === true,
    reason: normalizeText(reason, 300) || "",
    category: normalizeText(category, 60) || "",
    attempts: outcome === "skipped" ? 0 : attempts === 2 ? 2 : 1,
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
    if (stage.provider && stage.attempts === 2) metrics.stage_retries += 1;
    const ran = stage.status !== "skipped";
    if (stage.provider && stage.metrics.tokens) {
      anyTokens = true;
      for (const key of ["input", "output", "total"]) {
        if (stage.metrics.tokens[key] === null) metrics.tokens[key] = null;
        else if (metrics.tokens[key] !== null) metrics.tokens[key] += stage.metrics.tokens[key];
      }
      if (!stage.metrics.tokens.complete) metrics.tokens_complete = false;
    } else if (stage.provider && ran) {
      metrics.tokens_complete = false;
    }
    // Timing and retry metrics describe model work only. A stage that reached a provider without
    // reporting a measurement makes the total unknown, never a smaller measured number.
    if (!stage.provider) continue;
    for (const key of ["elapsed_ms", "request_retries", "output_repairs"]) {
      const measurable = ran;
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
// and successful, no required stage left unfinished, and an independent validation that actually
// succeeded. A mandatory stage is judged by its outcome rather than by its required flag, which a
// failed stage could simply omit.
function buildReport(stages = []) {
  const outcomes = (Array.isArray(stages) ? stages : []).map(stageOutcome);
  const ids = outcomes.map((stage) => stage.id);
  const byId = new Map(outcomes.map((stage) => [stage.id, stage]));
  const wellFormed = ids.every((id) => id !== "") &&
    new Set(ids).size === ids.length &&
    MANDATORY_STAGES.every((id) => byId.get(id)?.status === "success");
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
  // Success needs both sides to agree: the producer has to claim it and the stages have to prove it.
  // A missing or unknown status is malformed, so it reads as failed.
  return parsed.status === "success" ? report : { ...report, status: "failed" };
}

function stageIds(report) {
  return (report?.stages ?? []).map((stage) => stage.id);
}

module.exports = {
  MANDATORY_STAGES, REPORT_VERSION,
  buildReport, normalizeStageMetrics, parseReport, stageIds, stageOutcome,
};
