"use strict";

const { exactKeys } = require("./validation");

const APP = "github-actions";
const SCHEMA_VERSION = "review-outcome-v1";
const MARKER = "ironrdp-pr-automation-review-outcome:";

function encodeReviewOutcome({ headSha, outcome } = {}) {
  if (!/^[0-9a-f]{40}$/.test(headSha || "") || !["findings", "no-findings"].includes(outcome)) {
    throw new Error("invalid review outcome");
  }
  return `${MARKER} ${JSON.stringify({
    schema_version: SCHEMA_VERSION,
    head_sha: headSha,
    outcome,
  })}`;
}

function parseReviewOutcome(text, expectedSha) {
  if (typeof text !== "string" || Buffer.byteLength(text, "utf8") > 4096) return null;
  const lines = text.split(/\r?\n/).map((entry) => entry.trim()).filter((entry) => entry.startsWith(MARKER));
  if (lines.length !== 1) return null;
  let parsed;
  try { parsed = JSON.parse(lines[0].slice(MARKER.length)); } catch { return null; }
  if (!exactKeys(parsed, ["schema_version", "head_sha", "outcome"]) ||
      parsed.schema_version !== SCHEMA_VERSION || parsed.head_sha !== expectedSha ||
      !["findings", "no-findings"].includes(parsed.outcome)) return null;
  return parsed.outcome;
}

function trustedReviewOutcome(run, expectedSha) {
  if (run?.app?.slug !== APP || run.head_sha !== expectedSha ||
      run.external_id !== expectedSha || run.conclusion !== "success") return null;
  return parseReviewOutcome(run.output?.summary, expectedSha);
}

module.exports = {
  MARKER, SCHEMA_VERSION, encodeReviewOutcome, parseReviewOutcome, trustedReviewOutcome,
};
