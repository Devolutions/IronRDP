"use strict";

const { REPO_PATH, SHA, exactKeys, invalid, normalizeText, parseJson } = require("./validation");

const SCHEMA_VERSION = "reviewer-v1";

function lineSet(lines) {
  if (lines instanceof Set) return lines;
  if (Array.isArray(lines) && lines.every(Number.isSafeInteger)) return new Set(lines);
  return null;
}

// A line range survives only when every line in it is part of this pull request's diff, because the
// only consumer of a range is an inline review comment. GitHub rejects the whole createReview call
// with a 422 when any comment targets a line outside the diff, which would abandon the review and
// every label write that follows it.
function linesAreValidated(path, start, end, changedLines) {
  const changed = lineSet(changedLines instanceof Map ? changedLines.get(path) : changedLines?.[path]);
  if (!changed || end - start >= changed.size) return false;
  for (let line = start; line <= end; line += 1) if (!changed.has(line)) return false;
  return true;
}

function validateHandoff(value, protocolReceived) {
  if (!exactKeys(value, ["received", "disposition", "rationale"]) ||
      typeof value.received !== "boolean" || value.received !== protocolReceived) return null;
  const rationale = normalizeText(value.rationale, 800);
  if (rationale === null) return null;
  if (protocolReceived
    ? !["accepted", "partially_accepted", "rejected"].includes(value.disposition) || rationale === ""
    : value.disposition !== "not_applicable") return null;
  return { received: value.received, disposition: value.disposition, rationale };
}

function validateReviewer(raw, {
  expectedSha, changedPaths = [], changedLines = {}, protocolReceived = false,
} = {}) {
  const value = parseJson(raw, 32768);
  if (!exactKeys(value, ["head_sha", "has_findings", "summary", "protocol_handoff", "findings"]) ||
      !SHA.test(value.head_sha) || value.head_sha !== expectedSha ||
      typeof value.has_findings !== "boolean" || !Array.isArray(value.findings) ||
      value.findings.length > 20) return invalid("invalid reviewer object");
  const summary = normalizeText(value.summary, 1000);
  if (summary === null || value.has_findings !== (value.findings.length > 0)) {
    return invalid("invalid reviewer summary or finding count");
  }
  const handoff = validateHandoff(value.protocol_handoff, protocolReceived);
  if (handoff === null) return invalid("invalid protocol handoff accountability");
  const paths = new Set(changedPaths);
  const findings = [];
  const keys = [
    "classification", "severity", "path", "start_line", "end_line", "rationale", "confidence",
    "protocol_compatibility", "public_api_compatibility",
  ];
  for (const finding of value.findings) {
    const path = typeof finding?.path === "string" ? finding.path : null;
    if (!exactKeys(finding, keys) || !["critical", "high", "medium", "low"].includes(finding.severity) ||
        !["blocking", "non_blocking", "question"].includes(finding.classification) ||
        path === null || Buffer.byteLength(path, "utf8") > 300 || !REPO_PATH.test(path) ||
        typeof finding.protocol_compatibility !== "boolean" ||
        typeof finding.public_api_compatibility !== "boolean" ||
        !Number.isFinite(finding.confidence) || finding.confidence < 0 || finding.confidence > 1) {
      continue;
    }
    const linesAreNull = finding.start_line === null && finding.end_line === null;
    const linesAreIntegers = Number.isSafeInteger(finding.start_line) && finding.start_line >= 1 &&
      Number.isSafeInteger(finding.end_line) && finding.end_line >= finding.start_line;
    if (!linesAreNull && !linesAreIntegers) continue;
    const rationale = normalizeText(finding.rationale, 1200);
    if (rationale === null || !paths.has(path)) continue;
    const locationIsValidated = linesAreNull ||
      linesAreValidated(path, finding.start_line, finding.end_line, changedLines);
    findings.push({
      classification: finding.classification, severity: finding.severity, path,
      start_line: locationIsValidated ? finding.start_line : null,
      end_line: locationIsValidated ? finding.end_line : null, rationale, confidence: finding.confidence,
      protocol_compatibility: finding.protocol_compatibility,
      public_api_compatibility: finding.public_api_compatibility,
    });
  }
  if (value.has_findings && findings.length === 0) return invalid("no publishable findings");
  const normalized = {
    head_sha: value.head_sha, has_findings: findings.length > 0, summary,
    protocol_handoff: handoff, findings,
  };
  if (Buffer.byteLength(JSON.stringify(normalized), "utf8") > 32768) return invalid("reviewer output too large");
  return { ok: true, status: "valid", schemaVersion: SCHEMA_VERSION, value: normalized };
}

module.exports = { SCHEMA_VERSION, linesAreValidated, validateReviewer };
