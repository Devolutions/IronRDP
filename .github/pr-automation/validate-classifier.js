"use strict";

const { SHA, exactKeys, invalid, normalizeText, parseJson } = require("./validation");
const { validateReviewerRoute } = require("./routing");

const SCHEMA_VERSION = "classifier-v3";
// Machine-readable classifier state persisted on the SHA-bound check, because the review route runs
// in a later workflow run and cannot read classifier job outputs.
const CHECK_STATE_MARKER = "ironrdp-pr-automation-state:";
const OVERLAP_URL = /^https:\/\/github\.com\/Devolutions\/IronRDP\/pull\/[1-9][0-9]*$/;

function isDocumentationPath(path) {
  const behavioral = /\.(?:rs|cs|[cm]?[jt]sx?|svelte|ya?ml|toml|json|lock|sh|ps1|py|rb|java)$/i;
  return !behavioral.test(path) && (
    /^docs\//i.test(path) ||
    /\.(?:md|mdx|rst)$/i.test(path) ||
    /(?:^|\/)(?:README|CHANGELOG|CONTRIBUTING|CODE_OF_CONDUCT)(?:\.[^/]*)?$/i.test(path) ||
    /(?:^|\/)LICENSE(?:-[^/]*)?$/i.test(path)
  );
}

function validateClassifier(raw, {
  expectedSha, changedPaths, documentationOnlyPaths, overlapCandidates, prNumber,
} = {}) {
  if (prNumber !== undefined && (!Number.isSafeInteger(prNumber) || prNumber < 1)) {
    return invalid("invalid classifier validation context");
  }
  const value = parseJson(raw, 4096);
  const required = [
    "head_sha", "risk", "technical_debt", "documentation_only", "cross_cutting", "overlap",
    "likely_non_legitimate", "non_legitimate_confidence", "non_legitimate_reason",
    "breaking_change_suspected", "breaking_change_rationale", "breaking_change_surface",
    "protocol_related", "summary",
  ];
  if (!exactKeys(value, required)) return invalid("invalid classifier object");
  if (!SHA.test(value.head_sha) || value.head_sha !== expectedSha) return invalid("classifier SHA mismatch");
  if (!["low", "medium", "high"].includes(value.risk) ||
      typeof value.technical_debt !== "boolean" || typeof value.documentation_only !== "boolean" ||
      typeof value.cross_cutting !== "boolean" ||
      typeof value.breaking_change_suspected !== "boolean" ||
      typeof value.protocol_related !== "boolean" ||
      typeof value.likely_non_legitimate !== "boolean" ||
      !Number.isFinite(value.non_legitimate_confidence) ||
      value.non_legitimate_confidence < 0 || value.non_legitimate_confidence > 1) {
    return invalid("invalid classifier primitive");
  }

  const overlapKeys = ["detected", "similar_pr_number", "similar_pr_url", "confidence", "rationale"];
  const overlap = value.overlap;
  if (!exactKeys(overlap, overlapKeys) || typeof overlap.detected !== "boolean" ||
      !Number.isFinite(overlap.confidence) || overlap.confidence < 0 || overlap.confidence > 1 ||
      !((Number.isSafeInteger(overlap.similar_pr_number) && overlap.similar_pr_number > 0) ||
        overlap.similar_pr_number === null) ||
      !(typeof overlap.similar_pr_url === "string" || overlap.similar_pr_url === null)) {
    return invalid("invalid overlap result");
  }
  const rationale = normalizeText(overlap.rationale, 500);
  const breakingRationale = normalizeText(value.breaking_change_rationale, 500);
  const breakingSurface = normalizeText(value.breaking_change_surface, 200);
  const nonLegitimateReason = normalizeText(value.non_legitimate_reason, 500);
  const summary = normalizeText(value.summary, 1000);
  if ([rationale, breakingRationale, breakingSurface, nonLegitimateReason, summary].some((text) => text === null)) {
    return invalid("invalid classifier text");
  }
  if (value.likely_non_legitimate
    ? value.non_legitimate_confidence < 0.9 || nonLegitimateReason === ""
    : value.non_legitimate_confidence !== 0 || nonLegitimateReason !== "") {
    return invalid("incoherent legitimacy signal");
  }
  if (overlap.similar_pr_url !== null && !OVERLAP_URL.test(overlap.similar_pr_url)) {
    return invalid("invalid overlap URL");
  }
  if (overlap.detected) {
    if (overlap.similar_pr_number === null || overlap.similar_pr_url === null ||
        overlap.confidence < 0.85 || overlap.similar_pr_number === prNumber ||
        !overlap.similar_pr_url.endsWith(`/pull/${overlap.similar_pr_number}`) ||
        !Array.isArray(overlapCandidates) || overlapCandidates.length > 30 ||
        !overlapCandidates.some((candidate) =>
          exactKeys(candidate, ["number", "url"]) &&
          candidate.number === overlap.similar_pr_number &&
          candidate.url === overlap.similar_pr_url)) {
      return invalid("invalid overlap reference");
    }
  } else if (overlap.similar_pr_number !== null || overlap.similar_pr_url !== null ||
      overlap.confidence !== 0 || rationale !== "") {
    return invalid("false overlap has reference");
  }
  const docsOnly = documentationOnlyPaths === undefined
    ? Array.isArray(changedPaths) && changedPaths.every((path) => typeof path === "string" && isDocumentationPath(path))
    : documentationOnlyPaths === true;
  if (value.documentation_only && !docsOnly) {
    return invalid("documentation-only conflicts with changed paths");
  }
  if (value.documentation_only && value.technical_debt) {
    return invalid("documentation-only conflicts with technical debt");
  }
  const normalized = {
    head_sha: value.head_sha, risk: value.risk, technical_debt: value.technical_debt,
    documentation_only: value.documentation_only, cross_cutting: value.cross_cutting, overlap: {
      detected: overlap.detected, similar_pr_number: overlap.similar_pr_number,
      similar_pr_url: overlap.similar_pr_url, confidence: overlap.confidence, rationale,
    },
    likely_non_legitimate: value.likely_non_legitimate,
    non_legitimate_confidence: value.non_legitimate_confidence,
    non_legitimate_reason: nonLegitimateReason,
    breaking_change_suspected: value.breaking_change_suspected,
    breaking_change_rationale: breakingRationale, breaking_change_surface: breakingSurface,
    protocol_related: value.protocol_related, summary,
  };
  if (Buffer.byteLength(JSON.stringify(normalized), "utf8") > 4096) return invalid("classifier output too large");
  return { ok: true, status: "valid", value: normalized };
}

function encodeCheckState({
  protocolRelated, risk, specialistReviewers, automaticReviewEligible = true,
} = {}) {
  if (typeof protocolRelated !== "boolean") throw new Error("protocolRelated must be a boolean");
  const route = validateReviewerRoute({ reviewers: specialistReviewers, protocolRelated, risk });
  if (!route.ok) throw new Error(route.reason);
  if (typeof automaticReviewEligible !== "boolean") {
    throw new Error("automaticReviewEligible must be a boolean");
  }
  return `${CHECK_STATE_MARKER} ${JSON.stringify({
    schema_version: SCHEMA_VERSION,
    protocol_related: protocolRelated,
    risk,
    specialist_reviewers: route.reviewers,
    automatic_review_eligible: automaticReviewEligible,
  })}`;
}

function parseCheckState(text) {
  if (typeof text !== "string" || Buffer.byteLength(text, "utf8") > 4096) return null;
  const line = text.split(/\r?\n/).map((entry) => entry.trim())
    .find((entry) => entry.startsWith(CHECK_STATE_MARKER));
  if (!line) return null;
  let parsed;
  try { parsed = JSON.parse(line.slice(CHECK_STATE_MARKER.length)); } catch { return null; }
  const keys = [
    "schema_version", "protocol_related", "risk", "specialist_reviewers", "automatic_review_eligible",
  ];
  if (!exactKeys(parsed, keys) || parsed.schema_version !== SCHEMA_VERSION ||
      typeof parsed.protocol_related !== "boolean" ||
      typeof parsed.automatic_review_eligible !== "boolean") return null;
  const route = validateReviewerRoute({
    reviewers: parsed.specialist_reviewers,
    protocolRelated: parsed.protocol_related,
    risk: parsed.risk,
  });
  if (!route.ok) return null;
  return {
    protocolRelated: parsed.protocol_related,
    risk: parsed.risk,
    specialistReviewers: route.reviewers,
    automaticReviewEligible: parsed.automatic_review_eligible,
  };
}

module.exports = {
  CHECK_STATE_MARKER, SCHEMA_VERSION,
  encodeCheckState, isDocumentationPath, parseCheckState, validateClassifier,
};
