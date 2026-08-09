"use strict";

const { SCHEMA_VERSION: CLASSIFIER_SCHEMA_VERSION } = require("./validate-classifier");
const { SCHEMA_VERSION: REVIEWER_SCHEMA_VERSION } = require("./validate-reviewer");
const { validateClassifier } = require("./validate-classifier");
const { validateReviewer } = require("./validate-reviewer");

const RISK = ["risk/low", "risk/medium", "risk/high", "risk/unknown"];
const AI_COUNTS = ["ai-reviewed/1", "ai-reviewed/2"];
const LEGITIMACY_MARKER = "<!-- ironrdp-pr-automation:legitimacy:v1 -->";
const DUPLICATE_MARKER = "<!-- ironrdp-pr-automation:duplicate -->";
const XL_MARKER = "<!-- ironrdp-pr-automation:xl -->";
const FORK_QUOTA_MARKER = "<!-- ironrdp-pr-automation:fork-llm-quota -->";
const GLOBAL_QUOTA_MARKER = "<!-- ironrdp-pr-automation:fork-llm-global-budget -->";
const ELIGIBLE_MERGED_PRS = 3;

function labelsOf(labels) {
  return new Set((labels || []).map((label) => typeof label === "string" ? label : label?.name).filter(Boolean));
}

// Shared review policy. The workflow evaluates it before spending an LLM call and the publication
// path evaluates it again against the labels present at write time, so the rule lives here instead
// of being restated in the workflow where the two copies could drift apart.
function reviewPolicyEligible({ labels, legitimacyStopped, protocolRelated } = {}) {
  const present = labelsOf(labels);
  if (present.has("ai-reviewed/2") || present.has("duplicate") || present.has("size/XL") ||
      legitimacyStopped === true) return false;
  // Risk gates the non-protocol route only. Risk measures how much human scrutiny a change needs,
  // not how much an automated review is worth, so a protocol-related change is always reviewed.
  if (protocolRelated === true) return true;
  return !(present.has("risk/low") && !present.has("breaking-change"));
}

function boundStatus(value, expectedSha, allowed) {
  return value && value.head_sha === expectedSha && allowed.includes(value.status) ? value.status : "unavailable";
}

function quotaComment(rateLimit) {
  if (rateLimit?.status !== "limited") return null;
  if (rateLimit.scope === "author" && Number.isSafeInteger(rateLimit.quota)) {
    return { kind: "fork-quota", marker: FORK_QUOTA_MARKER, quota: rateLimit.quota };
  }
  if (rateLimit.scope === "global") return { kind: "global-quota", marker: GLOBAL_QUOTA_MARKER };
  return null;
}

function deterministicLabelSets(deterministic) {
  if (!deterministic?.ok) return [];
  return [
    { owned: deterministic.ownedPathLabels || [], desired: deterministic.pathLabels || [] },
    { owned: deterministic.sizeLabels || [], desired: [deterministic.sizeLabel].filter(Boolean) },
    { owned: ["contributor/first-time"], desired: deterministic.firstTime ? ["contributor/first-time"] : [] },
  ];
}

function failedClassification(expectedSha, deterministic, reason, rateLimit, semverStatus) {
  const comment = quotaComment(rateLimit);
  return {
    ok: true, mode: "classification", expectedSha, failed: true, reason,
    labelSets: [
      ...deterministicLabelSets(deterministic),
      { owned: RISK, desired: [semverStatus === "suspected" ? "risk/high" : "risk/unknown"] },
      ...(semverStatus === "suspected"
        ? [{ owned: ["breaking-change"], desired: ["breaking-change"] }]
        : []),
    ],
    addLabels: ["maintainer-required"], comments: comment ? [comment] : [],
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: "Classification unavailable",
      summary: `Automated classification was unavailable: ${reason}. Maintainer review is required.`,
      machineState: { protocolRelated: false },
      conclusion: "neutral",
    },
  };
}

// A size/XL pull request is excluded from automated review before any model runs, so the classifier
// is never invoked and no model-derived label can be produced. The deterministic signals are still
// published together with the split guidance, and the check title is deliberately not
// "Classification complete" so the review gate refuses to open the review route.
function xlClassification(expectedSha, deterministic, semverStatus) {
  return {
    ok: true, mode: "classification", expectedSha, oversized: true,
    labelSets: [
      ...deterministicLabelSets(deterministic),
      ...(semverStatus === "unavailable"
        ? [] : [{ owned: ["breaking-change"], desired: semverStatus === "suspected" ? ["breaking-change"] : [] }]),
      { owned: RISK, desired: [semverStatus === "suspected" ? "risk/high" : "risk/unknown"] },
    ],
    addLabels: ["maintainer-required"],
    comments: [{ kind: "xl", marker: XL_MARKER }],
    // Duplicate and legitimacy verdicts are model-derived. No model ran, so a previously posted
    // verdict is neither confirmed nor refuted here and is left untouched.
    removeCommentMarkers: [],
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: "Deterministic labelling only",
      summary: "This pull request is too large for automated review, so no model was invoked.",
      machineState: { protocolRelated: false },
    },
  };
}

function resolveClassificationState({
  expectedSha, labels, deterministic, classifier, classificationGate, changedPaths, prNumber, semver, rateLimit, force,
} = {}) {
  const existing = labelsOf(labels);
  const forced = force === true;
  const failureRateLimit = forced ? undefined : rateLimit;
  if (typeof expectedSha !== "string") return { ok: false, reason: "missing expected SHA" };
  if (!forced && existing.has("ai-reviewed/2")) {
    return failedClassification(expectedSha, deterministic, "terminal AI review count", failureRateLimit);
  }
  const semverStatus = boundStatus(semver, expectedSha, ["suspected", "not-suspected"]);
  // Checked before the classifier is consulted: an oversized pull request never reaches a model, so
  // there is no classifier output to validate and no quota to charge.
  if (!forced && deterministic?.ok && deterministic.sizeLabel === "size/XL") {
    return xlClassification(expectedSha, deterministic, semverStatus);
  }
  if (!forced && rateLimit && rateLimit.status !== "allowed") {
    return failedClassification(expectedSha, deterministic, "fork LLM quota unavailable", failureRateLimit, semverStatus);
  }
  if (!deterministic?.ok) {
    const reason = deterministic?.reason || "deterministic analysis unavailable";
    return failedClassification(expectedSha, deterministic, reason, failureRateLimit, semverStatus);
  }
  if (!forced && classificationGate?.available === false) {
    const reason = classificationGate.reason || "classification gate unavailable";
    return failedClassification(expectedSha, deterministic, reason, failureRateLimit, semverStatus);
  }
  const classifierResult = validateClassifier(classifier, {
    expectedSha, changedPaths, documentationOnlyPaths: deterministic.documentationOnlyPaths, prNumber,
  });
  if (!classifierResult?.ok || classifierResult.value?.head_sha !== expectedSha) {
    const reason = classifierResult?.reason || "classifier output unavailable";
    return failedClassification(expectedSha, deterministic, reason, failureRateLimit, semverStatus);
  }
  if (semverStatus === "unavailable") {
    return failedClassification(
      expectedSha, deterministic, "public API compatibility unavailable", failureRateLimit, semverStatus);
  }
  const model = classifierResult.value;
  const breaking = semverStatus === "suspected" || model.breaking_change_suspected;
  // cargo-semver-checks runs against the `ironrdp` facade, so every incompatibility it reports is a
  // core public API break and outranks the model. A break only the model suspects keeps the model's
  // judgement, except that a "low" verdict contradicts its own breaking-change signal.
  const risk = semverStatus === "suspected" ? "high"
    : model.breaking_change_suspected && model.risk === "low" ? "medium"
    : model.risk;
  const duplicate = model.duplicate.detected && model.duplicate.confidence >= 0.85;
  const isXl = deterministic.sizeLabel === "size/XL";
  const optional = [
    ["kind/technical-debt", model.technical_debt],
    ["kind/protocol", model.protocol_related],
    ["documentation", model.documentation_only],
    ["duplicate", duplicate],
  ];
  const labelSets = [
    { owned: RISK, desired: [`risk/${risk}`] },
    ...deterministicLabelSets(deterministic),
    { owned: ["scope/cross-cutting"], desired: model.cross_cutting ? ["scope/cross-cutting"] : [] },
    ...optional.map(([label, enabled]) => ({ owned: [label], desired: enabled ? [label] : [] })),
    { owned: ["breaking-change"], desired: breaking ? ["breaking-change"] : [] },
  ];
  const addLabels = ["maintainer-required"];
  const legitimacyStopped = model.likely_non_legitimate;
  const comments = [
    ...(duplicate ? [{
      kind: "duplicate", marker: DUPLICATE_MARKER,
      url: model.duplicate.similar_pr_url, rationale: model.duplicate.rationale,
    }] : []),
    ...(isXl && !forced ? [{ kind: "xl", marker: XL_MARKER }] : []),
    ...(legitimacyStopped ? [{
      kind: "legitimacy", marker: LEGITIMACY_MARKER, reason: model.non_legitimate_reason,
    }] : []),
  ];
  return {
    ok: true, mode: "classification", expectedSha, labelSets, addLabels, comments,
    dispatchReview: !forced,
    removeCommentMarkers: [
      ...(legitimacyStopped ? [] : [LEGITIMACY_MARKER]),
      // A later push can make a previously reported duplicate or XL verdict wrong, and stale
      // guidance would then contradict the labels this run just wrote.
      ...(duplicate ? [] : [DUPLICATE_MARKER]),
      ...(isXl && !forced ? [] : [XL_MARKER]),
    ],
    legitimacyStopped,
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: legitimacyStopped ? "Automation stopped" : "Classification complete",
      summary: legitimacyStopped
        ? "Validated human-triage classification is bound to this commit."
        : "Validated AI classification is bound to this commit.",
      machineState: {
        protocolRelated: model.protocol_related,
        automaticReviewEligible: !forced,
      },
    },
  };
}

function isExcludedHistory(pr) {
  const labels = labelsOf(pr.labels);
  return labels.has("trivial") || labels.has("reverted") || /^revert\b/i.test(pr.title || "");
}

function isBot(user) {
  return user?.type === "Bot" || /\[bot\]$/i.test(user?.login || "");
}

// Counts an author's merged pull requests, stopping at stopAt so a prolific contributor does not
// force a walk of the whole closed-PR history. Callers only ever compare against a threshold.
async function qualifyingMergedPrs({ github, owner, repo, authorNodeId, currentPrNumber, stopAt }) {
  let merged = 0;
  for await (const response of github.paginate.iterator(github.rest.pulls.list, {
    owner, repo, state: "closed", sort: "updated", direction: "desc", per_page: 100,
  })) {
    if (!Array.isArray(response?.data)) throw new Error("invalid pull request data");
    for (const pr of response.data) {
      if (!pr || typeof pr !== "object") throw new Error("invalid pull request data");
      if (pr.merged_at !== null && pr.merged_at !== undefined &&
          (typeof pr.merged_at !== "string" || Number.isNaN(Date.parse(pr.merged_at)))) {
        throw new Error("invalid pull request timestamp");
      }
      if (pr.number === currentPrNumber || !pr.merged_at || pr.user?.node_id !== authorNodeId ||
          isBot(pr.user) || isExcludedHistory(pr)) continue;
      merged += 1;
      if (merged >= stopAt) return merged;
    }
  }
  return merged;
}

async function contributorEligibility({ github, owner, repo, author, currentPrNumber }) {
  if (!author?.nodeId || author.type === "Bot" || /\[bot\]$/i.test(author.login || "")) {
    return { status: "ineligible", reason: "bot or missing immutable author" };
  }
  try {
    const merged = await qualifyingMergedPrs({
      github, owner, repo, authorNodeId: author.nodeId, currentPrNumber, stopAt: ELIGIBLE_MERGED_PRS,
    });
    return { status: merged >= ELIGIBLE_MERGED_PRS ? "eligible" : "ineligible", merged };
  } catch {
    return { status: "unavailable", reason: "GitHub API unavailable" };
  }
}

function resolveReviewState({
  expectedSha, labels, reviewer, changedPaths, changedLines, gate, contributor,
  rateLimit, protocolStatus, protocolReason, reviewerReason, evidenceReason, force, reviewMarkerId,
} = {}) {
  const existing = labelsOf(labels);
  const forced = force === true;
  const fail = (reason, report = false) => {
    const comment = forced ? null : quotaComment(rateLimit);
    return {
      ok: true, mode: "review", expectedSha, failed: true, reason,
      labelSets: [], addLabels: ["maintainer-required"], comments: comment ? [comment] : [],
      ...(report ? { check: {
        name: "AI automated review", externalId: `${REVIEWER_SCHEMA_VERSION}:${expectedSha}`,
        title: "Automated review unavailable",
        summary: `Automated review was unavailable: ${reason}. Maintainer review is required.`,
        conclusion: "neutral",
      } } : {}),
    };
  };
  if (typeof expectedSha !== "string") return { ok: false, reason: "missing expected SHA" };
  if (forced) {
    if (gate?.force !== true || gate.head_sha !== expectedSha) return fail("forced review gate unavailable");
    if (typeof reviewMarkerId !== "string" || !/^[1-9]\d{0,19}$/.test(reviewMarkerId)) {
      return fail("forced review marker unavailable");
    }
  } else {
    if (existing.has("ai-reviewed/2")) return fail("terminal AI review count");
    if (rateLimit && rateLimit.status !== "allowed") return fail("fork LLM quota unavailable");
    if (!gate?.ok || gate.head_sha !== expectedSha || gate.classificationCheck !== true ||
        gate.ciGreen !== true) {
      const reason = gate?.reason ? `review gate unavailable: ${gate.reason}` : "review gate unavailable";
      return fail(reason);
    }
    if (contributor?.status === "ineligible") {
      const reason = Number.isSafeInteger(contributor.merged)
        ? `contributor history ineligible (merged: ${contributor.merged}, required: ${ELIGIBLE_MERGED_PRS})`
        : `contributor history ineligible${contributor.reason ? `: ${contributor.reason}` : ""}`;
      return fail(reason);
    }
    if (contributor?.status !== "eligible") {
      const reason = contributor?.reason
        ? `contributor history unavailable: ${contributor.reason}`
        : "contributor history unavailable";
      return fail(reason);
    }
    if (existing.has("ai-reviewed/1") && gate.secondReviewEligible !== true) {
      return fail("second review is not eligible");
    }
    if (!reviewPolicyEligible({
      labels, legitimacyStopped: gate.legitimacyStopped, protocolRelated: gate.protocolRelated,
    })) return fail("review is not eligible");
  }
  if (evidenceReason) return fail(evidenceReason, true);
  // A protocol-related review is only publishable when the protocol stage produced a validated
  // handoff; anything else fails closed to humans.
  if (!["valid", "not_applicable"].includes(protocolStatus)) {
    return fail(protocolReason || "protocol handoff unavailable", true);
  }
  const reviewerResult = validateReviewer(reviewer, {
    expectedSha, changedPaths, changedLines, protocolReceived: protocolStatus === "valid",
  });
  if (!reviewerResult?.ok || reviewerResult.value?.head_sha !== expectedSha) {
    return fail(reviewerReason || reviewerResult?.reason || "reviewer unavailable", true);
  }
  const nextCount = existing.has("ai-reviewed/2") ? "ai-reviewed/2"
    : existing.has("ai-reviewed/1") ? "ai-reviewed/2"
    : "ai-reviewed/1";
  const hasFindings = reviewerResult.value.has_findings;
  const reviewMarker = `<!-- ironrdp-pr-automation:review:${expectedSha}` +
    `${forced ? `:force:${reviewMarkerId}` : ""} -->`;
  return {
    ok: true, mode: "review", expectedSha, labelSets: [{ owned: AI_COUNTS, desired: [nextCount] }],
    addLabels: nextCount === "ai-reviewed/2" || !hasFindings ? ["maintainer-required"] : [],
    removeLabels: nextCount === "ai-reviewed/1" && hasFindings ? ["maintainer-required"] : [],
    comments: hasFindings ? [{ kind: "review", marker: reviewMarker,
      review: reviewerResult.value }] : [],
    check: { name: "AI automated review", externalId: `${REVIEWER_SCHEMA_VERSION}:${expectedSha}` },
    reviewerSchemaVersion: REVIEWER_SCHEMA_VERSION,
  };
}

module.exports = {
  AI_COUNTS, DUPLICATE_MARKER, FORK_QUOTA_MARKER, GLOBAL_QUOTA_MARKER, LEGITIMACY_MARKER, RISK,
  XL_MARKER, ELIGIBLE_MERGED_PRS, contributorEligibility, isExcludedHistory, qualifyingMergedPrs,
  resolveClassificationState, resolveReviewState, reviewPolicyEligible,
};
