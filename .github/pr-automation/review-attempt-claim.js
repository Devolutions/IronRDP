"use strict";

const crypto = require("node:crypto");

const { SHA, exactKeys, isPlainObject, normalizeText } = require("./validation");
const { validateReviewerRoute } = require("./routing");

const CLAIM_STATE_MARKER = "ironrdp-pr-automation-review-claim:";
const CLAIM_SCHEMA_VERSION = "review-claim-v1";
const MAX_AUTOMATIC_ATTEMPTS = 2;
const RECOVERY_DELAY_SECONDS = 120;
const TERMINAL_STATUSES = new Set(["published", "exhausted"]);
const ACTIVE_WORKFLOW_STATUSES = new Set(["queued", "in_progress", "pending", "requested", "waiting"]);

function validOwner(value) {
  return isPlainObject(value) && exactKeys(value, ["run_id", "run_attempt"]) &&
    Number.isSafeInteger(value.run_id) && value.run_id > 0 &&
    Number.isSafeInteger(value.run_attempt) && value.run_attempt > 0;
}

function canonicalRecoveryIdentity({
  prNumber, headSha, baseSha, selectedReviewers, requiredReviewers, protocolRelated, risk, evidenceMaxBytes, workflowSha,
} = {}) {
  if (!Number.isSafeInteger(prNumber) || prNumber < 1 ||
      !SHA.test(headSha || "") || !SHA.test(baseSha || "") || !SHA.test(workflowSha || "") ||
      !Array.isArray(selectedReviewers) || !Array.isArray(requiredReviewers) ||
      typeof protocolRelated !== "boolean" || !Number.isSafeInteger(evidenceMaxBytes) || evidenceMaxBytes <= 0) {
    return null;
  }
  const route = validateReviewerRoute({
    reviewers: selectedReviewers, selectedReviewers, requiredReviewers,
    protocolRelated, risk,
  });
  if (!route.ok) return null;
  return JSON.stringify({
    base_sha: baseSha,
    evidence_max_bytes: evidenceMaxBytes,
    head_sha: headSha,
    pr_number: prNumber,
    protocol_related: protocolRelated,
    required_reviewers: route.requiredReviewers,
    risk,
    selected_reviewers: route.selectedReviewers,
    workflow_sha: workflowSha,
  });
}

function recoveryFingerprint(identity) {
  return typeof identity === "string" && identity.length <= 1024
    ? crypto.createHash("sha256").update(identity).digest("hex")
    : null;
}

function parseClaim(value) {
  if (typeof value !== "string" || Buffer.byteLength(value, "utf8") > 64 * 1024) return null;
  const line = value.split(/\r?\n/).map((entry) => entry.trim())
    .find((entry) => entry.startsWith(CLAIM_STATE_MARKER));
  if (!line || Buffer.byteLength(line, "utf8") > 1024) return null;
  let parsed;
  try { parsed = JSON.parse(line.slice(CLAIM_STATE_MARKER.length)); } catch { return null; }
  if (!exactKeys(parsed, ["schema_version", "fingerprint", "attempt", "owner", "status", "reason"]) ||
      parsed.schema_version !== CLAIM_SCHEMA_VERSION || !/^[0-9a-f]{64}$/.test(parsed.fingerprint) ||
      !Number.isSafeInteger(parsed.attempt) || parsed.attempt < 1 || parsed.attempt > MAX_AUTOMATIC_ATTEMPTS ||
      !validOwner(parsed.owner) || !["claimed", "pending", "published", "exhausted"].includes(parsed.status) ||
      normalizeText(parsed.reason, 300) === null) return null;
  return {
    fingerprint: parsed.fingerprint,
    attempt: parsed.attempt,
    owner: { runId: parsed.owner.run_id, runAttempt: parsed.owner.run_attempt },
    status: parsed.status,
    reason: normalizeText(parsed.reason, 300),
  };
}

function hasClaimMarker(value) {
  return typeof value === "string" && value.includes(CLAIM_STATE_MARKER);
}

function encodeClaim(claim) {
  if (!claim || !/^[0-9a-f]{64}$/.test(claim.fingerprint) ||
      !Number.isSafeInteger(claim.attempt) || claim.attempt < 1 ||
      claim.attempt > MAX_AUTOMATIC_ATTEMPTS ||
      !validOwner({ run_id: claim.owner?.runId, run_attempt: claim.owner?.runAttempt }) ||
      !["claimed", "pending", "published", "exhausted"].includes(claim.status)) {
    throw new Error("invalid review attempt claim");
  }
  const reason = normalizeText(claim.reason, 300);
  if (reason === null) throw new Error("invalid review attempt claim reason");
  return `${CLAIM_STATE_MARKER} ${JSON.stringify({
    schema_version: CLAIM_SCHEMA_VERSION,
    fingerprint: claim.fingerprint,
    attempt: claim.attempt,
    owner: { run_id: claim.owner.runId, run_attempt: claim.owner.runAttempt },
    status: claim.status,
    reason,
  })}`;
}

function ownerMatches(left, right) {
  return left?.runId === right?.runId && left?.runAttempt === right?.runAttempt;
}

function activeOwner(ownerRun, owner) {
  return ownerRun?.run_attempt === owner.runAttempt &&
    ACTIVE_WORKFLOW_STATUSES.has(ownerRun.status);
}

function claimAttempt({ previous, fingerprint, owner, ownerRun, nextAttempt = false } = {}) {
  if (!/^[0-9a-f]{64}$/.test(fingerprint || "") ||
      !validOwner({ run_id: owner?.runId, run_attempt: owner?.runAttempt })) {
    return { ok: false, reason: "invalid review attempt claim input" };
  }
  if (!previous) {
    return {
      ok: true,
      claim: { fingerprint, attempt: 1, owner, status: "claimed", reason: "" },
      reclaimed: false,
    };
  }
  if (previous.fingerprint !== fingerprint) {
    return { ok: false, reason: "review attempt claim fingerprint changed" };
  }
  if (TERMINAL_STATUSES.has(previous.status)) {
    return { ok: false, reason: "automatic review attempt budget is complete" };
  }
  if (ownerMatches(previous.owner, owner)) {
    if (nextAttempt) {
      if (previous.attempt >= MAX_AUTOMATIC_ATTEMPTS) {
        return { ok: false, reason: "automatic review attempt budget is complete" };
      }
      return {
        ok: true,
        claim: {
          fingerprint,
          attempt: previous.attempt + 1,
          owner,
          status: "claimed",
          reason: "",
        },
        reclaimed: false,
      };
    }
    return { ok: true, claim: previous, reclaimed: false };
  }
  if (activeOwner(ownerRun, previous.owner)) {
    return { ok: false, reason: "automatic review attempt is owned by an active workflow" };
  }
  if (!ownerRun || !["completed", "cancelled"].includes(ownerRun.status) ||
      previous.attempt >= MAX_AUTOMATIC_ATTEMPTS) {
    return { ok: false, reason: "automatic review attempt budget is complete" };
  }
  return {
    ok: true,
    claim: {
      fingerprint,
      attempt: previous.attempt + 1,
      owner,
      status: "claimed",
      reason: "reclaimed after stale workflow owner",
    },
    reclaimed: true,
  };
}

function updateClaim({ claim, owner, status, reason = "" } = {}) {
  if (!claim || !ownerMatches(claim.owner, owner) ||
      !["pending", "published", "exhausted"].includes(status)) {
    return { ok: false, reason: "invalid review attempt claim update" };
  }
  const normalizedReason = normalizeText(reason, 300);
  if (normalizedReason === null) return { ok: false, reason: "invalid review attempt claim reason" };
  return {
    ok: true,
    claim: { ...claim, status, reason: normalizedReason },
  };
}

module.exports = {
  CLAIM_SCHEMA_VERSION, CLAIM_STATE_MARKER, MAX_AUTOMATIC_ATTEMPTS, RECOVERY_DELAY_SECONDS,
  canonicalRecoveryIdentity, claimAttempt, encodeClaim, hasClaimMarker, parseClaim, recoveryFingerprint, updateClaim,
};
