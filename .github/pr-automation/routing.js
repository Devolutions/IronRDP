"use strict";

// Persisted routes and aggregates use this canonical reviewer order.
const REVIEWER_ORDER = Object.freeze(["protocol", "skeptical", "code-compressor"]);
const REVIEWER_IDS = new Set(REVIEWER_ORDER);
const RISKS = new Set(["low", "medium", "high", "unknown"]);

function normalizeReviewerIds(value) {
  if (!Array.isArray(value) ||
      value.some((reviewer) => typeof reviewer !== "string" || !REVIEWER_IDS.has(reviewer)) ||
      new Set(value).size !== value.length) return null;
  const selected = new Set(value);
  return REVIEWER_ORDER.filter((reviewer) => selected.has(reviewer));
}

function requiredReviewerIds({ protocolRelated, risk } = {}) {
  if (typeof protocolRelated !== "boolean" || !RISKS.has(risk)) {
    return { ok: false, reason: "invalid required reviewer input" };
  }
  return {
    ok: true,
    reviewers: REVIEWER_ORDER.filter((reviewer) =>
      (reviewer === "protocol" && protocolRelated) ||
      (reviewer === "skeptical" && (risk === "medium" || risk === "high"))),
  };
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
  const required = requiredReviewerIds({ protocolRelated, risk });
  if (!required.ok) return required;
  const reviewers = REVIEWER_ORDER.filter((reviewer) => selected.has(reviewer));
  return {
    ok: true,
    reviewers,
    selectedReviewers: reviewers,
    requiredReviewers: required.reviewers,
  };
}

function validateReviewerRoute({
  reviewers, selectedReviewers = reviewers, requiredReviewers, protocolRelated, risk,
} = {}) {
  const canonical = normalizeReviewerIds(selectedReviewers);
  if (!canonical || canonical.some((reviewer, index) => reviewer !== selectedReviewers[index]) ||
      (reviewers !== undefined &&
      (canonical.length !== reviewers.length || canonical.some((reviewer, index) => reviewer !== reviewers[index])))) {
    return { ok: false, reason: "invalid persisted reviewer route" };
  }
  const mandatory = requiredReviewerIds({ protocolRelated, risk });
  const required = normalizeReviewerIds(requiredReviewers ?? mandatory.reviewers);
  if (!mandatory.ok || !required ||
      required.some((reviewer, index) => reviewer !== (requiredReviewers ?? mandatory.reviewers)[index]) ||
      required.some((reviewer) => !canonical.includes(reviewer)) ||
      mandatory.reviewers.some((reviewer) => !required.includes(reviewer))) {
    return { ok: false, reason: "incomplete persisted reviewer route" };
  }
  return {
    ok: true,
    reviewers: canonical,
    selectedReviewers: canonical,
    requiredReviewers: required,
  };
}

function labelsOf(labels) {
  return new Set((labels || []).map((label) => typeof label === "string" ? label : label?.name).filter(Boolean));
}

function reviewPolicyEligible({ labels, legitimacyStopped } = {}) {
  const present = labelsOf(labels);
  if (present.has("ai-reviewed/2") || present.has("duplicate") ||
      present.has("triage/legitimacy") || legitimacyStopped === true) return false;
  return true;
}

module.exports = {
  REVIEWER_ORDER,
  normalizeReviewerIds, requiredReviewerIds,
  resolveReviewerRoute,
  reviewPolicyEligible,
  validateReviewerRoute,
};
