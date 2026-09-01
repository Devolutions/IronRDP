"use strict";

// Persisted routes use this canonical sequential execution order.
const REVIEWER_ORDER = Object.freeze(["protocol", "skeptical", "code-compressor"]);
const REVIEWER_IDS = new Set(REVIEWER_ORDER);
const RISKS = new Set(["low", "medium", "high", "unknown"]);

function normalizeReviewerIds(value) {
  if (!Array.isArray(value) || value.length > REVIEWER_ORDER.length ||
      value.some((reviewer) => typeof reviewer !== "string" || !REVIEWER_IDS.has(reviewer)) ||
      new Set(value).size !== value.length) return null;
  const selected = new Set(value);
  return REVIEWER_ORDER.filter((reviewer) => selected.has(reviewer));
}

function resolveReviewerRoute({
  suggestedReviewers = [], deterministicReviewers = [], protocolRelated, risk,
} = {}) {
  const suggested = normalizeReviewerIds(suggestedReviewers);
  const deterministic = normalizeReviewerIds(deterministicReviewers);
  if (!suggested || !deterministic || typeof protocolRelated !== "boolean" || !RISKS.has(risk)) {
    return { ok: false, reason: "invalid reviewer route input" };
  }
  const selected = new Set([...suggested, ...deterministic]);
  if (protocolRelated) selected.add("protocol");
  if (risk === "medium" || risk === "high") selected.add("skeptical");
  return {
    ok: true,
    reviewers: REVIEWER_ORDER.filter((reviewer) => selected.has(reviewer)),
  };
}

function validateReviewerRoute({ reviewers, protocolRelated, risk } = {}) {
  const canonical = normalizeReviewerIds(reviewers);
  if (!canonical || canonical.some((reviewer, index) => reviewer !== reviewers[index])) {
    return { ok: false, reason: "invalid persisted reviewer route" };
  }
  const mandatory = resolveReviewerRoute({
    suggestedReviewers: [], deterministicReviewers: [], protocolRelated, risk,
  });
  if (!mandatory.ok || mandatory.reviewers.some((reviewer) => !canonical.includes(reviewer))) {
    return { ok: false, reason: "incomplete persisted reviewer route" };
  }
  return { ok: true, reviewers: canonical };
}

function labelsOf(labels) {
  return new Set((labels || []).map((label) => typeof label === "string" ? label : label?.name).filter(Boolean));
}

function reviewPolicyEligible({ labels, legitimacyStopped, protocolRelated } = {}) {
  const present = labelsOf(labels);
  const oversized = present.has("size/XXL") && !present.has("ai-review/allow-oversized");
  if (present.has("ai-reviewed/2") || present.has("duplicate") || oversized ||
      present.has("triage/legitimacy") || legitimacyStopped === true) return false;
  if (protocolRelated === true) return true;
  return !(present.has("risk/low") && !present.has("breaking-change"));
}

module.exports = {
  REVIEWER_ORDER,
  normalizeReviewerIds,
  resolveReviewerRoute,
  reviewPolicyEligible,
  validateReviewerRoute,
};
