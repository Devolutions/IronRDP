"use strict";

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
    if (gate.secondReviewEligible !== true) {
      reasons.push("An automated review has already run for this head; push a new commit before the next review.");
    }
    if (gate.policyEligible !== true) {
      const labels = new Set(Array.isArray(gate.labels) ? gate.labels : []);
      const policyReasonStart = reasons.length;
      if (labels.has("ai-reviewed/2")) reasons.push("The pull request has reached the two-review limit.");
      if (labels.has("duplicate")) reasons.push("The pull request is marked as a duplicate.");
      if (labels.has("triage/legitimacy") || gate.legitimacyStopped === true) {
        reasons.push("The pull request requires a maintainer legitimacy decision.");
      }
      if (reasons.length === policyReasonStart) {
        reasons.push("The pull request is not eligible under the automated review policy.");
      }
    }

    const contributor = gate.contributor;
    if (contributor?.status === "ineligible") {
      if (Number.isSafeInteger(contributor.merged)) {
        reasons.push(`The contributor has ${contributor.merged} qualifying merged pull requests; at least one is required.`);
      } else {
        reasons.push(`The contributor is not eligible for automated review${contributor.reason ? `: ${contributor.reason}` : ""}.`);
      }
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
