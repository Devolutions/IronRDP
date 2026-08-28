"use strict";

const {
  REPO_PATH, SHA, exactKeys, invalid, isBoundedArray, linesAreValidated, normalizeText, parseJson,
} = require("./validation");
const { REVIEWER_ORDER: REVIEWERS } = require("./routing");

const SCHEMA_VERSION = "final-review-v1";
const MAXIMUM_BYTES = 65536;
const MAXIMUM_CANDIDATES = 60;
const MAXIMUM_FINDINGS = 20;
const FINDING_ID = /^[a-z][a-z0-9-]{0,63}$/;
const REVIEWER_ORDER = new Map(REVIEWERS.map((reviewer, index) => [reviewer, index]));

function referenceKey(reference) {
  return `${reference.reviewer}\0${reference.finding_id}`;
}

function normalizeReference(value) {
  if (!exactKeys(value, ["reviewer", "finding_id"]) ||
      !REVIEWER_ORDER.has(value.reviewer) || !FINDING_ID.test(value.finding_id)) return null;
  return { reviewer: value.reviewer, finding_id: value.finding_id };
}

function sourceCategories(sources) {
  if (sources.length === 0) return ["general"];
  if (sources.some((source) => normalizeReference(source) === null)) {
    throw new Error("invalid validated finding source");
  }
  return [...new Set(sources.map((source) => source.reviewer))]
    .sort((left, right) => REVIEWER_ORDER.get(left) - REVIEWER_ORDER.get(right));
}

function provenancePrefix(sources) {
  return `[${sourceCategories(sources).join(" + ")}]`;
}

function aggregateCandidates(specialistAggregate, expectedSha) {
  const aggregate = specialistAggregate;
  if (!exactKeys(aggregate, ["head_sha", "reviewers"]) ||
      aggregate.head_sha !== expectedSha || !SHA.test(aggregate.head_sha) ||
      !isBoundedArray(aggregate.reviewers, REVIEWERS.length)) {
    return null;
  }
  const candidates = new Map();
  let previousReviewer = -1;
  const candidateKeys = [
    "id", "classification", "severity", "path", "start_line", "end_line", "title", "rationale",
    "confidence", "references",
  ];
  for (const review of aggregate.reviewers) {
    const reviewerIndex = REVIEWER_ORDER.get(review?.reviewer);
    if (reviewerIndex === undefined || reviewerIndex <= previousReviewer) return null;
    previousReviewer = reviewerIndex;
    if (review.status === "failed") {
      if (!exactKeys(review, ["reviewer", "status", "reason"]) ||
          !normalizeText(review.reason, 300)) return null;
      continue;
    }
    if (review.status !== "valid" ||
        !exactKeys(review, ["reviewer", "status", "summary", "findings"]) ||
        !normalizeText(review.summary, 1000) ||
        !isBoundedArray(review.findings, MAXIMUM_FINDINGS)) return null;
    for (const candidate of review.findings) {
      if (!exactKeys(candidate, candidateKeys) ||
          !["blocking", "non_blocking", "question"].includes(candidate.classification) ||
          !["critical", "high", "medium", "low"].includes(candidate.severity) ||
          !Array.isArray(candidate.references)) return null;
      const reference = normalizeReference({
        reviewer: review.reviewer,
        finding_id: candidate.id,
      });
      if (reference === null || candidates.has(referenceKey(reference))) return null;
      candidates.set(referenceKey(reference), reference);
    }
  }
  return candidates;
}

function normalizeDispositions(entries, candidates) {
  if (!isBoundedArray(entries, MAXIMUM_CANDIDATES) || entries.length !== candidates.size) return null;
  const byCandidate = new Map();
  for (const entry of entries) {
    if (!exactKeys(entry, ["reviewer", "finding_id", "disposition", "rationale"]) ||
        !["accepted", "refined", "rejected"].includes(entry.disposition)) return null;
    const reference = normalizeReference({
      reviewer: entry.reviewer,
      finding_id: entry.finding_id,
    });
    const rationale = normalizeText(entry.rationale, 800);
    if (reference === null || !rationale) return null;
    const key = referenceKey(reference);
    if (!candidates.has(key) || byCandidate.has(key)) return null;
    const normalized = { ...reference, disposition: entry.disposition, rationale };
    byCandidate.set(key, normalized);
  }
  if ([...candidates.keys()].some((key) => !byCandidate.has(key))) return null;
  return byCandidate;
}

function normalizeFinding(finding, changedPaths, changedLines, dispositions, referencedCandidates) {
  const keys = [
    "classification", "severity", "path", "start_line", "end_line", "title", "rationale",
    "confidence", "sources",
  ];
  if (!exactKeys(finding, keys) ||
      !["blocking", "non_blocking", "question"].includes(finding.classification) ||
      !["critical", "high", "medium", "low"].includes(finding.severity) ||
      typeof finding.path !== "string" || Buffer.byteLength(finding.path, "utf8") > 300 ||
      finding.path.includes("\\") || !REPO_PATH.test(finding.path) || !changedPaths.has(finding.path) ||
      !Number.isFinite(finding.confidence) || finding.confidence < 0 || finding.confidence > 1 ||
      !isBoundedArray(finding.sources, MAXIMUM_CANDIDATES)) return null;

  const linesAreNull = finding.start_line === null && finding.end_line === null;
  const linesAreIntegers = Number.isSafeInteger(finding.start_line) && finding.start_line >= 1 &&
    Number.isSafeInteger(finding.end_line) && finding.end_line >= finding.start_line;
  if (!linesAreNull && !linesAreIntegers) return null;

  const title = normalizeText(finding.title, 200);
  const rationale = normalizeText(finding.rationale, 1200);
  if (!title || !rationale) return null;

  const sources = [];
  const localSources = new Set();
  for (const source of finding.sources) {
    const reference = normalizeReference(source);
    if (reference === null) return null;
    const key = referenceKey(reference);
    const disposition = dispositions.get(key);
    if (!disposition || disposition.disposition === "rejected" ||
        localSources.has(key) || referencedCandidates.has(key)) return null;
    localSources.add(key);
    referencedCandidates.add(key);
    sources.push(reference);
  }
  sources.sort((left, right) =>
    REVIEWER_ORDER.get(left.reviewer) - REVIEWER_ORDER.get(right.reviewer) ||
    left.finding_id.localeCompare(right.finding_id));

  const locationIsValidated = linesAreNull ||
    linesAreValidated(finding.path, finding.start_line, finding.end_line, changedLines);
  return {
    classification: finding.classification,
    severity: finding.severity,
    path: finding.path,
    start_line: locationIsValidated ? finding.start_line : null,
    end_line: locationIsValidated ? finding.end_line : null,
    title,
    rationale,
    confidence: finding.confidence,
    sources,
  };
}

function validateFinalReview(raw, {
  expectedSha, changedPaths = [], changedLines = {}, specialistAggregate,
} = {}) {
  const candidates = aggregateCandidates(specialistAggregate, expectedSha);
  if (candidates === null) return invalid("validated specialist findings unavailable");

  const value = parseJson(raw, MAXIMUM_BYTES);
  if (!exactKeys(value, ["head_sha", "summary", "candidate_dispositions", "findings"]) ||
      !SHA.test(value.head_sha) || value.head_sha !== expectedSha ||
      !isBoundedArray(value.findings, MAXIMUM_FINDINGS)) {
    return invalid("invalid final review object");
  }
  const summary = normalizeText(value.summary, 1000);
  if (!summary) return invalid("invalid final review summary");

  const normalizedDispositions = normalizeDispositions(value.candidate_dispositions, candidates);
  if (normalizedDispositions === null) return invalid("invalid specialist candidate dispositions");

  const findings = [];
  const referencedCandidates = new Set();
  const paths = new Set(changedPaths);
  for (const finding of value.findings) {
    const normalized = normalizeFinding(
      finding, paths, changedLines, normalizedDispositions, referencedCandidates,
    );
    if (normalized === null) return invalid("invalid final review finding");
    findings.push(normalized);
  }

  for (const [key, disposition] of normalizedDispositions) {
    const referenced = referencedCandidates.has(key);
    if (disposition.disposition === "rejected" ? referenced : !referenced) {
      return invalid("specialist disposition contradicts final findings");
    }
  }

  const normalized = {
    head_sha: value.head_sha,
    summary,
    findings,
    has_findings: findings.length > 0,
  };
  if (Buffer.byteLength(JSON.stringify(normalized), "utf8") > MAXIMUM_BYTES) {
    return invalid("final review output too large");
  }
  return { ok: true, status: "valid", schemaVersion: SCHEMA_VERSION, value: normalized };
}

function validateNormalizedFinalReview(value, expectedSha) {
  if (!exactKeys(value, [
    "head_sha", "summary", "findings", "has_findings",
  ]) || value.head_sha !== expectedSha || !SHA.test(value.head_sha) ||
      typeof value.has_findings !== "boolean" ||
      value.has_findings !== (Array.isArray(value.findings) && value.findings.length > 0) ||
      !isBoundedArray(value.findings, MAXIMUM_FINDINGS)) return invalid("invalid validated final review");
  for (const finding of value.findings) {
    if (!exactKeys(finding, [
      "classification", "severity", "path", "start_line", "end_line", "title", "rationale",
      "confidence", "sources",
    ])) return invalid("invalid validated final review finding");
    try {
      provenancePrefix(finding.sources);
    } catch {
      return invalid("invalid validated final review source");
    }
  }
  return { ok: true, status: "valid", schemaVersion: SCHEMA_VERSION, value };
}

module.exports = {
  MAXIMUM_CANDIDATES, MAXIMUM_FINDINGS, REVIEWERS, SCHEMA_VERSION,
  provenancePrefix, sourceCategories, validateFinalReview,
  validateNormalizedFinalReview,
};
