"use strict";

const { REVIEWER_ORDER } = require("./routing");
const { escapeMarkdown, reducedCoverageText } = require("./write-state");

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

function stageFailures(stages) {
  return stages.flatMap((stage) => {
    const finalReason = stage.reason && stage.category
      ? `${stage.reason} (${stage.category})`
      : stage.reason || stage.category;
    return stage.status === "failed" ? [[stage.id, finalReason || "unknown"]] : [];
  });
}

function diagnostics(report, outcome, maxStages, maxTextLength, includeReasons) {
  const stages = report.stages.slice(0, maxStages);
  const omittedStages = report.stages.length - stages.length;
  const metrics = report.metrics;
  const stageOutcomes = stages.map((stage) => [stage.id, stage.status]);
  if (omittedStages > 0) {
    stageOutcomes.push(["additional stages", `${omittedStages} omitted to bound output`]);
  }
  const totalMetrics = table(["Input tokens", "Output tokens", "Total tokens", "Cumulative elapsed (seconds)", "Request retries", "Output repairs"], [[
    ...tokenColumns(metrics.tokens, metrics.tokens_complete),
    seconds(metrics.elapsed_ms), metric(metrics.request_retries), metric(metrics.output_repairs),
  ]], maxTextLength);
  const stageRows = stages.filter((stage) => stage.provider).map((stage) => [
    stage.id, stage.status, ...tokenColumns(stage.metrics.tokens),
    seconds(stage.metrics.elapsed_ms), metric(stage.metrics.request_retries),
    metric(stage.metrics.output_repairs),
  ]);
  const stageMetrics = table(
    ["Stage", "Status", "Input tokens", "Output tokens", "Total tokens",
      "Cumulative elapsed (seconds)", "Request retries", "Output repairs"],
    stageRows, maxTextLength,
  );
  const reasons = (() => {
    if (!includeReasons) return "";
    const failures = stageFailures(stages);
    if (omittedStages > 0) failures.push(["additional stages", `${omittedStages} omitted to bound output`]);
    return failures.length === 0
      ? "No stage failure was reported."
      : table(["Stage", "Reason"], failures, maxTextLength);
  })();
  return [
    `Review outcome: **${outcome}**.`,
    "",
    "### Stage outcomes",
    table(["Stage", "Status"], stageOutcomes, maxTextLength),
    "",
    "### Metrics",
    totalMetrics,
    "",
    "Token totals count repeated context across requests and repairs.",
    "",
    "### LLM stage metrics",
    stageMetrics,
    ...(includeReasons ? ["", "### Failed stages", reasons] : []),
  ].join("\n");
}

function renderReviewReport({ report, outcome, summaryUrl, reducedCoverage = [] }) {
  const checkDiagnostics = diagnostics(report, outcome, MAX_CHECK_STAGES, MAX_CHECK_TEXT_LENGTH, false);
  const workflowDiagnostics = diagnostics(report, outcome, MAX_WORKFLOW_STAGES, MAX_WORKFLOW_TEXT_LENGTH, true);
  const heading = {
    complete: "Automated review complete",
    "reduced-coverage": "Automated review completed with reduced coverage",
    unavailable: "Automated review unavailable",
  }[outcome];
  const outcomeText = outcome === "complete" || outcome === "reduced-coverage"
    ? "Validated automated review is bound to this commit"
    : "Automated review is unavailable. Maintainer review is required";
  const coverage = outcome.endsWith("reduced-coverage")
    ? ` with reduced coverage:${reducedCoverageText(
      reducedCoverage.map((reviewer) => text(reviewer, MAX_CHECK_TEXT_LENGTH)),
    )}`
    : "";
  return {
    title: heading,
    checkSummary: `${outcomeText}${coverage}.\n\n${checkDiagnostics}\n\n[View the workflow summary](${summaryUrl})`,
    workflowSummary: `# Automated review\n\n${workflowDiagnostics}`,
  };
}

module.exports = { renderReviewReport };
