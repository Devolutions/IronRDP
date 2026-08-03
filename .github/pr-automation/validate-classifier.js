"use strict";

const { SHA, exactKeys, invalid, normalizeText, parseJson } = require("./validation");

const SCHEMA_VERSION = "classifier-v1";
// Machine-readable classifier state persisted on the SHA-bound check, because the review route runs
// in a later workflow run and cannot read classifier job outputs.
const CHECK_STATE_MARKER = "ironrdp-pr-automation-state:";
const DUPLICATE_URL = /^https:\/\/github\.com\/Devolutions\/IronRDP\/pull\/[1-9][0-9]*$/;

function isDocumentationPath(path) {
  const behavioral = /\.(?:rs|cs|[cm]?[jt]sx?|svelte|ya?ml|toml|json|lock|sh|ps1|py|rb|java)$/i;
  return !behavioral.test(path) && (
    /^docs\//i.test(path) ||
    /\.(?:md|mdx|rst)$/i.test(path) ||
    /(?:^|\/)(?:README|CHANGELOG|CONTRIBUTING|CODE_OF_CONDUCT)(?:\.[^/]*)?$/i.test(path) ||
    /(?:^|\/)LICENSE(?:-[^/]*)?$/i.test(path)
  );
}

function validateClassifier(raw, { expectedSha, changedPaths, documentationOnlyPaths, prNumber } = {}) {
  const value = parseJson(raw, 4096);
  const required = [
    "schema_version", "head_sha", "risk", "technical_debt", "documentation_only", "duplicate",
    "likely_non_legitimate", "non_legitimate_confidence", "non_legitimate_reason",
    "breaking_change_suspected", "breaking_change_rationale", "breaking_change_surface",
    "protocol_related", "summary",
  ];
  if (!exactKeys(value, required)) return invalid("invalid classifier object");
  if (value.schema_version !== "1" || !SHA.test(value.head_sha) || value.head_sha !== expectedSha) {
    return invalid("classifier schema version or SHA mismatch");
  }
  if (!["low", "medium", "high"].includes(value.risk) ||
      typeof value.technical_debt !== "boolean" || typeof value.documentation_only !== "boolean" ||
      typeof value.breaking_change_suspected !== "boolean" ||
      typeof value.protocol_related !== "boolean" ||
      typeof value.likely_non_legitimate !== "boolean" ||
      !Number.isFinite(value.non_legitimate_confidence) ||
      value.non_legitimate_confidence < 0 || value.non_legitimate_confidence > 1) {
    return invalid("invalid classifier primitive");
  }

  const duplicateKeys = ["detected", "similar_pr_number", "similar_pr_url", "confidence", "rationale"];
  const duplicate = value.duplicate;
  if (!exactKeys(duplicate, duplicateKeys) || typeof duplicate.detected !== "boolean" ||
      !Number.isFinite(duplicate.confidence) || duplicate.confidence < 0 || duplicate.confidence > 1 ||
      !((Number.isSafeInteger(duplicate.similar_pr_number) && duplicate.similar_pr_number > 0) ||
        duplicate.similar_pr_number === null) ||
      !(typeof duplicate.similar_pr_url === "string" || duplicate.similar_pr_url === null)) {
    return invalid("invalid duplicate result");
  }
  const rationale = normalizeText(duplicate.rationale, 500);
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
  if (duplicate.similar_pr_url !== null && !DUPLICATE_URL.test(duplicate.similar_pr_url)) {
    return invalid("invalid duplicate URL");
  }
  if (duplicate.detected) {
    if (duplicate.similar_pr_number === null || duplicate.similar_pr_url === null ||
        duplicate.confidence < 0.85 || duplicate.similar_pr_number === prNumber ||
        !duplicate.similar_pr_url.endsWith(`/pull/${duplicate.similar_pr_number}`)) {
      return invalid("invalid duplicate reference");
    }
  } else if (duplicate.similar_pr_number !== null || duplicate.similar_pr_url !== null ||
      duplicate.confidence !== 0 || rationale !== "") {
    return invalid("false duplicate has reference");
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
    schema_version: value.schema_version, head_sha: value.head_sha, risk: value.risk, technical_debt: value.technical_debt,
    documentation_only: value.documentation_only, duplicate: {
      detected: duplicate.detected, similar_pr_number: duplicate.similar_pr_number,
      similar_pr_url: duplicate.similar_pr_url, confidence: duplicate.confidence, rationale,
    },
    likely_non_legitimate: value.likely_non_legitimate,
    non_legitimate_confidence: value.non_legitimate_confidence,
    non_legitimate_reason: nonLegitimateReason,
    breaking_change_suspected: value.breaking_change_suspected,
    breaking_change_rationale: breakingRationale, breaking_change_surface: breakingSurface,
    protocol_related: value.protocol_related, summary,
  };
  if (Buffer.byteLength(JSON.stringify(normalized), "utf8") > 4096) return invalid("classifier output too large");
  return { ok: true, status: "valid", schemaVersion: SCHEMA_VERSION, value: normalized };
}

function encodeCheckState({ protocolRelated } = {}) {
  if (typeof protocolRelated !== "boolean") throw new Error("protocolRelated must be a boolean");
  return `${CHECK_STATE_MARKER} ${JSON.stringify({ schema_version: SCHEMA_VERSION, protocol_related: protocolRelated })}`;
}

function parseCheckState(text) {
  if (typeof text !== "string" || Buffer.byteLength(text, "utf8") > 4096) return null;
  const line = text.split(/\r?\n/).map((entry) => entry.trim())
    .find((entry) => entry.startsWith(CHECK_STATE_MARKER));
  if (!line) return null;
  let parsed;
  try { parsed = JSON.parse(line.slice(CHECK_STATE_MARKER.length)); } catch { return null; }
  if (!exactKeys(parsed, ["schema_version", "protocol_related"]) ||
      parsed.schema_version !== SCHEMA_VERSION || typeof parsed.protocol_related !== "boolean") return null;
  return { protocolRelated: parsed.protocol_related };
}

module.exports = {
  CHECK_STATE_MARKER, SCHEMA_VERSION,
  encodeCheckState, isDocumentationPath, parseCheckState, validateClassifier,
};
