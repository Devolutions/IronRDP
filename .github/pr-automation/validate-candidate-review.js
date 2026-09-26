"use strict";

const {
  REPO_PATH, SHA, exactKeys, invalid, linesAreValidated, normalizeText, parseJson,
} = require("./validation");
const { REVIEWER_ORDER: REVIEWERS } = require("./routing");

const SEVERITIES = new Set(["critical", "high", "medium", "low"]);
const FINDING_ID = /^[a-z][a-z0-9-]{0,63}$/;
const PROTOCOL_ID = /^(?:MS|MC)-[A-Z0-9]+$/;
const SECTION = /^[0-9]+(?:\.[0-9]+)*$/;
const FINDING_KEYS = [
  "id", "question", "severity", "path", "start_line", "end_line", "title", "rationale",
  "confidence", "references",
];
const REFERENCE_KEYS = ["protocol_id", "section", "heading"];

function normalizeReference(value) {
  if (!exactKeys(value, REFERENCE_KEYS)) return null;
  const protocolId = normalizeText(value.protocol_id, 40);
  const section = normalizeText(value.section, 80);
  const heading = normalizeText(value.heading, 200);
  if (!protocolId || !PROTOCOL_ID.test(protocolId) || !section || !SECTION.test(section) || !heading) {
    return null;
  }
  return { protocol_id: protocolId, section, heading };
}

function normalizeCandidateReview(raw, {
  expectedSha, expectedReviewer, changedPaths = [], changedLines = {},
} = {}) {
  const value = parseJson(raw, 32768);
  if (!exactKeys(value, ["head_sha", "reviewer", "summary", "findings"]) ||
      !SHA.test(value.head_sha) || value.head_sha !== expectedSha ||
      !REVIEWERS.includes(value.reviewer) || value.reviewer !== expectedReviewer ||
      !Array.isArray(value.findings) || value.findings.length > 20) {
    return invalid("invalid candidate review object");
  }

  const summary = normalizeText(value.summary, 1000);
  if (!summary) return invalid("invalid candidate review summary");

  const paths = new Set(changedPaths);
  const ids = new Set();
  const findings = [];
  for (const finding of value.findings) {
    if (!exactKeys(finding, FINDING_KEYS)) return invalid("invalid candidate finding");

    const id = normalizeText(finding.id, 64);
    const path = typeof finding.path === "string" ? finding.path : null;
    const title = normalizeText(finding.title, 200);
    const rationale = normalizeText(finding.rationale, 1200);
    if (!id || !FINDING_ID.test(id) || ids.has(id) ||
        typeof finding.question !== "boolean" || !SEVERITIES.has(finding.severity) ||
        path === null || Buffer.byteLength(path, "utf8") > 300 || path.includes("\\") ||
        !REPO_PATH.test(path) || !paths.has(path) || !title || !rationale ||
        !Number.isFinite(finding.confidence) || finding.confidence < 0 || finding.confidence > 1 ||
        !Array.isArray(finding.references) || finding.references.length > 5 ||
        (value.reviewer !== "protocol" && finding.references.length !== 0)) {
      return invalid("invalid candidate finding");
    }

    const linesAreNull = finding.start_line === null && finding.end_line === null;
    const linesAreIntegers = Number.isSafeInteger(finding.start_line) && finding.start_line >= 1 &&
      Number.isSafeInteger(finding.end_line) && finding.end_line >= finding.start_line;
    if (!linesAreNull && !linesAreIntegers) return invalid("invalid candidate finding lines");

    const references = [];
    for (const reference of finding.references) {
      const normalized = normalizeReference(reference);
      if (normalized === null) return invalid("invalid candidate finding reference");
      references.push(normalized);
    }

    const locationIsValidated = linesAreNull ||
      linesAreValidated(path, finding.start_line, finding.end_line, changedLines);
    ids.add(id);
    findings.push({
      id,
      question: finding.question,
      severity: finding.severity,
      path,
      start_line: locationIsValidated ? finding.start_line : null,
      end_line: locationIsValidated ? finding.end_line : null,
      title,
      rationale,
      confidence: finding.confidence,
      references,
    });
  }

  const normalized = {
    head_sha: value.head_sha,
    reviewer: value.reviewer,
    summary,
    findings,
  };
  if (Buffer.byteLength(JSON.stringify(normalized), "utf8") > 32768) {
    return invalid("candidate review output too large");
  }
  return { ok: true, status: "valid", value: normalized };
}

module.exports = {
  normalizeCandidateReview, validateCandidateReview: normalizeCandidateReview,
};
