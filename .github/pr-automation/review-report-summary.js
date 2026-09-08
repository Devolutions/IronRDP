"use strict";

const { REVIEWER_ORDER } = require("./routing");

const MAX_CHECK_STAGES = REVIEWER_ORDER.length + 4;
const MAX_WORKFLOW_STAGES = 64;
const MAX_TEXT_LENGTH = 300;

function escapeMarkdown(value) {
  return String(value).replace(/\\/g, "\\\\")
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;").replace(/'/g, "&#39;").replace(/`/g, "&#96;")
    .replace(/@(?=[\w-])/g, "`@`").replace(/(?<!&)#(?=\d)/g, "`#`")
    .replace(/[[\]()!*_~|]/g, "\\$&");
}

function text(value) {
  return escapeMarkdown(String(value ?? "").slice(0, MAX_TEXT_LENGTH) || "unknown");
}

function metric(value) {
  return value === null ? "unavailable" : value === undefined ? "unknown" : String(value);
}

function tokens(tokens, complete = tokens?.complete) {
  if (tokens === null) return "unavailable";
  if (tokens === undefined) return "unknown";
  const values = ["input", "output", "total"].map((name) => metric(tokens[name]));
  return `${values.join("/")} ${complete ? "" : "(partial)"}`.trim();
}

function table(headers, rows) {
  const line = (columns) => `| ${columns.map(text).join(" | ")} |`;
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

function diagnostics(report, outcome, maxStages) {
  const stages = report.stages.slice(0, maxStages);
  const omittedStages = report.stages.length - stages.length;
  const attempts = failureAttempts(stages);
  if (omittedStages > 0) attempts.push(["additional stages", "unknown", `${omittedStages} omitted to bound output`]);
  const metrics = report.metrics;
  const failedAttempts = attempts.length === 0
    ? "No stage failure was reported."
    : table(["Stage", "Attempt", "Reason"], attempts);
  const totalMetrics = table(["Metric", "Total"], [
    ["Tokens", tokens(metrics.tokens, metrics.tokens_complete)],
    ["Elapsed (ms)", metric(metrics.elapsed_ms)],
    ["Request retries", metric(metrics.request_retries)],
    ["Output repairs", metric(metrics.output_repairs)],
    ["Stage retries", metric(metrics.stage_retries)],
  ]);
  const stageRows = stages.map((stage) => [
    stage.id, stage.status, stage.attempts, tokens(stage.metrics.tokens),
    metric(stage.metrics.elapsed_ms), metric(stage.metrics.request_retries),
    metric(stage.metrics.output_repairs),
  ]);
  if (omittedStages > 0) {
    stageRows.push(["additional stages", "unknown", "unknown", "unknown", "unknown", "unknown",
      `${omittedStages} omitted to bound output`]);
  }
  const stageMetrics = table(
    ["Stage", "Status", "Attempts", "Tokens", "Elapsed (ms)", "Request retries", "Output repairs"],
    stageRows,
  );
  return [
    `Review outcome: **${outcome}**.`,
    "",
    "### Failed stage attempts",
    failedAttempts,
    "",
    "### Metrics",
    totalMetrics,
    "",
    "### Per-stage metrics",
    stageMetrics,
  ].join("\n");
}

function renderReviewReport({ report, outcome, detail, summaryUrl }) {
  const checkDiagnostics = diagnostics(report, outcome, MAX_CHECK_STAGES);
  const workflowDiagnostics = diagnostics(report, outcome, MAX_WORKFLOW_STAGES);
  const heading = {
    complete: "Automated review complete",
    recovered: "Automated review recovered",
    unavailable: "Automated review unavailable",
  }[outcome];
  const outcomeText = outcome === "complete"
    ? "Validated automated review is bound to this commit."
    : outcome === "recovered"
      ? "Validated automated review was produced after stage recovery."
      : `Automated review is unavailable: ${text(detail || "review unavailable")}. Maintainer review is required.`;
  return {
    title: heading,
    checkSummary: `${outcomeText}\n\n${checkDiagnostics}\n\n[View the workflow summary](${summaryUrl})`,
    workflowSummary: `# Automated review\n\n${workflowDiagnostics}`,
  };
}

module.exports = { renderReviewReport };
