"use strict";

const { REVIEWER_ORDER } = require("./routing");
const { escapeMarkdown } = require("./write-state");

const MAX_CHECK_STAGES = REVIEWER_ORDER.length + 4;
const MAX_WORKFLOW_STAGES = 64;
const MAX_CHECK_TEXT_LENGTH = 250;
const MAX_WORKFLOW_TEXT_LENGTH = 300;

function text(value, maxTextLength) {
  return escapeMarkdown(String(value ?? "").slice(0, maxTextLength) || "unknown");
}

function metric(value) {
  return value === null ? "unavailable" : value === undefined ? "unknown" : String(value);
}

function tokenColumns(tokens, complete = tokens?.complete) {
  if (tokens === null) return ["unavailable", "unavailable", "unavailable"];
  if (tokens === undefined) return ["unknown", "unknown", "unknown"];
  const values = ["input", "output", "total"].map((name) => metric(tokens[name]));
  values[2] = `${values[2]}${complete ? "" : " (partial)"}`;
  return values;
}

function seconds(milliseconds) {
  return milliseconds === null ? "unavailable" :
    milliseconds === undefined ? "unknown" : String(milliseconds / 1000);
}

function table(headers, rows, maxTextLength) {
  const line = (columns) => `| ${columns.map((column) => text(column, maxTextLength)).join(" | ")} |`;
  return [line(headers), `| ${headers.map(() => "---").join(" | ")} |`, ...rows.map(line)].join("\n");
}

function failureAttempts(stages) {
  return stages.flatMap((stage) => {
    const finalReason = stage.reason && stage.category
      ? `${stage.reason} (${stage.category})`
      : stage.reason || stage.category;
    return [
      ...(stage.previous_reason ? [[stage.id, "first attempt", stage.previous_reason]] : []),
      ...(stage.status === "failed" ? [[stage.id, "final attempt", finalReason || "unknown"]] : []),
    ];
  });
}

function diagnostics(report, outcome, maxStages, maxTextLength, includeReasons) {
  const stages = report.stages.slice(0, maxStages);
  const omittedStages = report.stages.length - stages.length;
  const metrics = report.metrics;
  const stageOutcomes = stages.map((stage) => [stage.id, stage.status, stage.attempts]);
  if (omittedStages > 0) {
    stageOutcomes.push(["additional stages", "unknown", `${omittedStages} omitted to bound output`]);
  }
  const totalMetrics = table(["Input tokens", "Output tokens", "Total tokens", "Cumulative elapsed (seconds)", "Request retries", "Output repairs", "Stage recoveries"], [[
    ...tokenColumns(metrics.tokens, metrics.tokens_complete),
    seconds(metrics.elapsed_ms), metric(metrics.request_retries), metric(metrics.output_repairs),
    metric(metrics.stage_retries),
  ]], maxTextLength);
  const stageRows = stages.filter((stage) => stage.provider).map((stage) => [
    stage.id, stage.status, stage.attempts, ...tokenColumns(stage.metrics.tokens),
    seconds(stage.metrics.elapsed_ms), metric(stage.metrics.request_retries),
    metric(stage.metrics.output_repairs),
  ]);
  const stageMetrics = table(
    ["Stage", "Status", "Attempts", "Input tokens", "Output tokens", "Total tokens",
      "Cumulative elapsed (seconds)", "Request retries", "Output repairs"],
    stageRows, maxTextLength,
  );
  const reasons = (() => {
    if (!includeReasons) return "";
    const attempts = failureAttempts(stages);
    if (omittedStages > 0) attempts.push(["additional stages", "unknown", `${omittedStages} omitted to bound output`]);
    return attempts.length === 0
      ? "No stage failure was reported."
      : table(["Stage", "Attempt", "Reason"], attempts, maxTextLength);
  })();
  return [
    `Review outcome: **${outcome}**.`,
    "",
    "### Stage outcomes",
    table(["Stage", "Status", "Attempts"], stageOutcomes, maxTextLength),
    "",
    "### Metrics",
    totalMetrics,
    "",
    "Token totals count repeated context across requests and repairs.",
    "",
    "### LLM stage metrics",
    stageMetrics,
    ...(includeReasons ? ["", "### Failed stage attempts", reasons] : []),
  ].join("\n");
}

function reducedCoverageText(reducedCoverage) {
  return ` with reduced coverage: optional reviewer${reducedCoverage.length === 1 ? "" : "s"} ` +
    `${reducedCoverage.map((reviewer) => text(reviewer, MAX_CHECK_TEXT_LENGTH)).join(", ")} ` +
    `${reducedCoverage.length === 1 ? "was" : "were"} unavailable.`;
}

function renderReviewReport({ report, outcome, summaryUrl, reducedCoverage = [] }) {
  const checkDiagnostics = diagnostics(report, outcome, MAX_CHECK_STAGES, MAX_CHECK_TEXT_LENGTH, false);
  const workflowDiagnostics = diagnostics(report, outcome, MAX_WORKFLOW_STAGES, MAX_WORKFLOW_TEXT_LENGTH, true);
  const heading = {
    complete: "Automated review complete",
    recovered: "Automated review recovered",
    "reduced-coverage": "Automated review completed with reduced coverage",
    "recovered-reduced-coverage": "Automated review recovered with reduced coverage",
    unavailable: "Automated review unavailable",
  }[outcome];
  const outcomeText = outcome === "complete" || outcome === "reduced-coverage"
    ? "Validated automated review is bound to this commit"
    : outcome === "recovered" || outcome === "recovered-reduced-coverage"
      ? "Validated automated review was produced after stage recovery"
      : "Automated review is unavailable. Maintainer review is required";
  const coverage = outcome.endsWith("reduced-coverage") ? reducedCoverageText(reducedCoverage) : "";
  return {
    title: heading,
    checkSummary: `${outcomeText}${coverage}.\n\n${checkDiagnostics}\n\n[View the workflow summary](${summaryUrl})`,
    workflowSummary: `# Automated review\n\n${workflowDiagnostics}`,
  };
}

module.exports = { renderReviewReport };
