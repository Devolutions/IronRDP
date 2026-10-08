"use strict";

const { exactKeys } = require("./validation");

const APP = "github-actions";
const SCHEMA_VERSION = "review-outcome-v1";
const COUNTED_SCHEMA_VERSION = "review-outcome-v2";
const MARKER = "ironrdp-pr-automation-review-outcome:";
const { AI_COUNTS } = require("./review-count");

function reviewMarkerPrefix(headSha) {
  return `<!-- ironrdp-pr-automation:review:${headSha}`;
}

function encodeReviewOutcome({ headSha, outcome, nextReviewCount, reviewMarker } = {}) {
  if (!/^[0-9a-f]{40}$/.test(headSha || "") || !["findings", "no-findings"].includes(outcome)) {
    throw new Error("invalid review outcome");
  }
  if (nextReviewCount !== undefined &&
      (!AI_COUNTS.includes(nextReviewCount) ||
       !validReviewMarker(reviewMarker, headSha))) throw new Error("invalid counted review outcome");
  return `${MARKER} ${JSON.stringify({
    schema_version: nextReviewCount === undefined ? SCHEMA_VERSION : COUNTED_SCHEMA_VERSION,
    head_sha: headSha,
    outcome,
    ...(nextReviewCount === undefined ? {} : { next_review_count: nextReviewCount, review_marker: reviewMarker }),
  })}`;
}

function validReviewMarker(marker, headSha) {
  if (typeof marker !== "string") return false;
  const prefix = reviewMarkerPrefix(headSha);
  return marker.startsWith(prefix) &&
    /^(?: -->|:force:[1-9]\d{0,19} -->)(?![\s\S])/.test(marker.slice(prefix.length));
}

function parseReviewReceipt(text, expectedSha) {
  if (typeof text !== "string" || Buffer.byteLength(text, "utf8") > 4096) return null;
  const lines = text.split(/\r?\n/).map((entry) => entry.trim()).filter((entry) => entry.startsWith(MARKER));
  if (lines.length !== 1) return null;
  let parsed;
  try { parsed = JSON.parse(lines[0].slice(MARKER.length)); } catch { return null; }
  if (parsed?.schema_version === SCHEMA_VERSION) {
    if (!exactKeys(parsed, ["schema_version", "head_sha", "outcome"])) return null;
  } else if (parsed?.schema_version === COUNTED_SCHEMA_VERSION) {
    if (!exactKeys(parsed, ["schema_version", "head_sha", "outcome", "next_review_count", "review_marker"]) ||
        !AI_COUNTS.includes(parsed.next_review_count) ||
        !validReviewMarker(parsed.review_marker, expectedSha)) return null;
  } else return null;
  if (parsed.head_sha !== expectedSha || !["findings", "no-findings"].includes(parsed.outcome)) return null;
  return parsed;
}

function parseReviewOutcome(text, expectedSha) {
  return parseReviewReceipt(text, expectedSha)?.outcome ?? null;
}

function trustedReviewReceipt(run, expectedSha) {
  if (run?.app?.slug !== APP || run.head_sha !== expectedSha ||
      run.external_id !== expectedSha || run.conclusion !== "success") return null;
  return parseReviewReceipt(run.output?.summary, expectedSha);
}

function trustedReviewOutcome(run, expectedSha) {
  return trustedReviewReceipt(run, expectedSha)?.outcome ?? null;
}

module.exports = {
  MARKER, SCHEMA_VERSION, COUNTED_SCHEMA_VERSION, encodeReviewOutcome, parseReviewOutcome,
  trustedReviewOutcome, trustedReviewReceipt, reviewMarkerPrefix,
};
