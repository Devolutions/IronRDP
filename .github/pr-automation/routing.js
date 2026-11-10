"use strict";

const { AI_COUNTS, reviewCount } = require("./review-count");

// Persisted routes and aggregates use this canonical reviewer order. It is also the matrix order, so
// required and correctness reviewers start first when more than three specialists are selected.
const REVIEWER_ORDER = Object.freeze([
  "protocol", "skeptical", "intent-drift", "intent-review", "test", "dependency", "code-compressor",
  "rust-style",
]);
const REVIEWER_IDS = new Set(REVIEWER_ORDER);
// Findings each specialist may report. The narrow conditional specialists get a smaller budget, so
// the maximum-shaped general review still fits the runtime's model message limit.
const SPECIALIST_FINDING_LIMITS = Object.freeze({
  protocol: 20,
  skeptical: 20,
  "intent-drift": 10,
  "intent-review": 10,
  test: 10,
  dependency: 10,
  "code-compressor": 20,
  "rust-style": 10,
});
const MAXIMUM_CANDIDATES = REVIEWER_ORDER.reduce(
  (total, reviewer) => total + SPECIALIST_FINDING_LIMITS[reviewer], 0);
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

// Suspected overlap is approximate and must not suppress code review.
function reviewPolicyEligible({ labels, legitimacyStopped } = {}) {
  const present = labelsOf(labels);
  const count = reviewCount(present);
  if (count === undefined || count === AI_COUNTS.at(-1) ||
      present.has("triage/legitimacy") || legitimacyStopped === true) return false;
  return true;
}

module.exports = {
  MAXIMUM_CANDIDATES,
  REVIEWER_ORDER,
  SPECIALIST_FINDING_LIMITS,
  normalizeReviewerIds,
  resolveReviewerRoute,
  reviewPolicyEligible,
  validateReviewerRoute,
};
