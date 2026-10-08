"use strict";

const { AI_COUNTS, reviewCount } = require("./review-count");
function reviewSkipReasons({ gate, gateResult, rateLimit, rateLimitResult } = {}) {
  const reasons = [];

  if (gateResult !== "success") {
    reasons.push(`The review gate job did not complete successfully${gateResult ? ` (${gateResult})` : ""}.`);
  } else if (!gate || typeof gate !== "object") {
    reasons.push("The review gate output is unavailable.");
  } else if (typeof gate.reason === "string" && gate.reason) {
    reasons.push(`The review gate is unavailable: ${gate.reason}.`);
  } else if (gate.ok !== true) {
    if (gate.classificationCheck !== true) {
      reasons.push("A successful, review-eligible AI classification is not available for this head.");
    }
    if (gate.ciGreen !== true) reasons.push("CI has not succeeded for this head.");
    if (gate.nextReviewEligible !== true) {
      reasons.push("An automated review has already run for this head; push a new commit before the next review.");
    }
    if (gate.policyEligible !== true) {
      const labels = new Set(Array.isArray(gate.labels) ? gate.labels : []);
      const policyReasonStart = reasons.length;
      if (reviewCount(labels) === AI_COUNTS.at(-1)) reasons.push("The pull request has reached the three-review limit.");
      if (reviewCount(labels) === undefined) reasons.push("The review count labels are ambiguous.");
      if (labels.has("triage/legitimacy") || gate.legitimacyStopped === true) {
        reasons.push("The pull request is awaiting its green exact-head CI handoff for legitimacy triage.");
      }
      if (reasons.length === policyReasonStart) {
        reasons.push("The pull request is not eligible under the automated review policy.");
      }
    }

    const contributor = gate.contributor;
    if (contributor?.status === "bot") {
      reasons.push("The pull request was opened by a bot account.");
    } else if (contributor?.status !== "eligible" && contributor?.status !== "forced") {
      reasons.push(`Contributor eligibility is unavailable${contributor?.reason ? `: ${contributor.reason}` : ""}.`);
    }
  }

  if (rateLimitResult !== "success") {
    reasons.push(`The fork automation quota job did not complete successfully${rateLimitResult ? ` (${rateLimitResult})` : ""}.`);
  } else if (!rateLimit || typeof rateLimit !== "object") {
    reasons.push("The fork automation quota output is unavailable.");
  } else if (rateLimit.status === "limited") {
    const usage = Number.isSafeInteger(rateLimit.count) && Number.isSafeInteger(rateLimit.quota)
      ? ` (${rateLimit.count} counted, limit ${rateLimit.quota})`
      : "";
    reasons.push(`The daily fork automation quota is exhausted${usage}.`);
  } else if (rateLimit.status !== "allowed") {
    reasons.push(`The fork automation quota is unavailable${rateLimit.reason ? `: ${rateLimit.reason}` : ""}.`);
  }

  if (reasons.length === 0) {
    reasons.push("The workflow's automated review conditions were not satisfied.");
  }

  return reasons;
}

module.exports = { reviewSkipReasons };
