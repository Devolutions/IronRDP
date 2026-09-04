"use strict";

const { SCHEMA_VERSION: CLASSIFIER_SCHEMA_VERSION, validateClassifier } = require("./validate-classifier");
const { validateNormalizedFinalReview } = require("./validate-final-review");
const { resolveReviewerRoute, reviewPolicyEligible, validateReviewerRoute } = require("./routing");

const RISK = ["risk/low", "risk/medium", "risk/high", "risk/unknown"];
const AI_COUNTS = ["ai-reviewed/1", "ai-reviewed/2"];
const LEGITIMACY_LABEL = "triage/legitimacy";
const OVERSIZED_REVIEW_LABEL = "ai-review/allow-oversized";
const LEGITIMACY_MARKER_PREFIX = "<!-- ironrdp-pr-automation:legitimacy:v2:";
const DUPLICATE_MARKER = "<!-- ironrdp-pr-automation:duplicate -->";
const OVERSIZED_MARKER = "<!-- ironrdp-pr-automation:oversized -->";
const LEGACY_XL_MARKER = "<!-- ironrdp-pr-automation:xl -->";
const FORK_QUOTA_MARKER = "<!-- ironrdp-pr-automation:fork-llm-quota -->";
const GLOBAL_QUOTA_MARKER = "<!-- ironrdp-pr-automation:fork-llm-global-budget -->";
const EVIDENCE_LIMIT_MARKER = "<!-- ironrdp-pr-automation:evidence-limit -->";
const CONTRIBUTOR_INELIGIBLE_MARKER = "<!-- ironrdp-pr-automation:contributor-ineligible -->";
const EVIDENCE_LIMIT_REASON = /^pull request diff exceeds the (1|4) MiB evidence limit$/;
const ELIGIBLE_MERGED_PRS = 1;
const ELIGIBLE_ASSOCIATIONS = new Set(["OWNER", "MEMBER"]);

function labelsOf(labels) {
  return new Set((labels || []).map((label) => typeof label === "string" ? label : label?.name).filter(Boolean));
}

function boundStatus(value, expectedSha, allowed) {
  return value && value.head_sha === expectedSha && allowed.includes(value.status) ? value.status : "unavailable";
}

function quotaComment(rateLimit) {
  if (rateLimit?.status !== "limited") return null;
  if (rateLimit.scope === "global") return { kind: "global-quota", marker: GLOBAL_QUOTA_MARKER };
  return null;
}

function evidenceLimitComment(reason) {
  const match = typeof reason === "string" ? EVIDENCE_LIMIT_REASON.exec(reason) : null;
  return match
    ? { kind: "evidence-limit", marker: EVIDENCE_LIMIT_MARKER, limitMiB: Number(match[1]) }
    : null;
}

function deterministicLabelSets(deterministic) {
  if (!deterministic?.ok) return [];
  return [
    { owned: deterministic.ownedPathLabels || [], desired: deterministic.pathLabels || [] },
    { owned: deterministic.sizeLabels || [], desired: [deterministic.sizeLabel].filter(Boolean) },
    { owned: ["contributor/first-time"], desired: deterministic.firstTime ? ["contributor/first-time"] : [] },
  ];
}

function classificationMachineState({
  risk = "unknown", protocolRelated = false,
  automaticReviewEligible = false,
} = {}) {
  const route = resolveReviewerRoute({
    deterministicReviewers: ["code-compressor"], protocolRelated, risk,
  });
  if (!route.ok) return null;
  return {
    protocolRelated,
    risk,
    specialistReviewers: route.reviewers,
    automaticReviewEligible,
  };
}

function failedClassification(expectedSha, deterministic, reason, rateLimit, semverStatus) {
  const comments = [quotaComment(rateLimit), evidenceLimitComment(reason)].filter(Boolean);
  return {
    ok: true, mode: "classification", expectedSha, failed: true, reason,
    labelSets: [
      ...deterministicLabelSets(deterministic),
      { owned: RISK, desired: [semverStatus === "suspected" ? "risk/high" : "risk/unknown"] },
      ...(semverStatus === "suspected"
        ? [{ owned: ["breaking-change"], desired: ["breaking-change"] }]
        : []),
    ],
    addLabels: ["maintainer-required"], comments,
    removeCommentMarkers: [
      ...(comments.some((comment) => comment.kind === "evidence-limit") ? [] : [EVIDENCE_LIMIT_MARKER]),
      FORK_QUOTA_MARKER,
      ...(comments.some((comment) => comment.kind === "global-quota") ? [] : [GLOBAL_QUOTA_MARKER]),
    ],
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: "Classification unavailable",
      summary: `Automated classification was unavailable: ${reason}. Maintainer review is required.`,
      machineState: classificationMachineState({
        risk: semverStatus === "suspected" ? "high" : "unknown",
      }),
      conclusion: "neutral",
    },
  };
}

function resolveClassificationState({
  expectedSha, labels, deterministic, classifier, classificationGate,
  classifierReason, changedPaths, duplicateCandidates, prNumber, semver, rateLimit, force,
} = {}) {
  const existing = labelsOf(labels);
  const forced = force === true;
  const failureRateLimit = forced ? undefined : rateLimit;
  if (typeof expectedSha !== "string") return { ok: false, reason: "missing expected SHA" };
  if (!forced && existing.has("ai-reviewed/2")) {
    return failedClassification(expectedSha, deterministic, "terminal AI review count", failureRateLimit);
  }
  const semverStatus = boundStatus(semver, expectedSha, ["suspected", "not-suspected"]);
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
    expectedSha, changedPaths, documentationOnlyPaths: deterministic.documentationOnlyPaths,
    duplicateCandidates, prNumber,
  });
  if (!classifierResult?.ok || classifierResult.value?.head_sha !== expectedSha) {
    const reason = classifierReason || classifierResult?.reason || "classifier output unavailable";
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
  const machineState = classificationMachineState({
    risk,
    protocolRelated: model.protocol_related,
    automaticReviewEligible: !forced,
  });
  if (!machineState) {
    return failedClassification(
      expectedSha, deterministic, "reviewer routing unavailable", failureRateLimit, semverStatus);
  }
  const duplicate = model.duplicate.detected && model.duplicate.confidence >= 0.85;
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
  const legitimacyStopped = model.likely_non_legitimate;
  const addLabels = [
    "maintainer-required",
    ...(legitimacyStopped ? [LEGITIMACY_LABEL] : []),
  ];
  const comments = [
    ...(duplicate ? [{
      kind: "duplicate", marker: DUPLICATE_MARKER,
      url: model.duplicate.similar_pr_url, rationale: model.duplicate.rationale,
    }] : []),
  ];
  const auditComments = [
    ...(legitimacyStopped ? [{
      kind: "legitimacy", marker: `${LEGITIMACY_MARKER_PREFIX}${expectedSha} -->`,
      sha: expectedSha, reason: model.non_legitimate_reason,
    }] : []),
  ];
  return {
    ok: true, mode: "classification", expectedSha, labelSets, addLabels, comments, auditComments,
    dispatchReview: !forced,
    removeCommentMarkers: [
      // A later push can make a previously reported duplicate or oversized verdict wrong, and stale
      // guidance would then contradict the labels this run just wrote.
      ...(duplicate ? [] : [DUPLICATE_MARKER]),
      EVIDENCE_LIMIT_MARKER,
      FORK_QUOTA_MARKER,
      GLOBAL_QUOTA_MARKER,
      OVERSIZED_MARKER,
      LEGACY_XL_MARKER,
    ],
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: legitimacyStopped ? "Automation stopped" : "Classification complete",
      summary: legitimacyStopped
        ? "Validated human-triage classification is bound to this commit."
        : "Validated AI classification is bound to this commit.",
      machineState,
    },
  };
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
      if (pr.number === currentPrNumber || !pr.merged_at || pr.base?.ref !== "master" ||
          pr.user?.node_id !== authorNodeId || isBot(pr.user)) continue;
      merged += 1;
      if (merged >= stopAt) return merged;
    }
  }
  return merged;
}

async function contributorEligibility({ github, owner, repo, author, currentPrNumber }) {
  if (author?.type === "Bot" || /\[bot\]$/i.test(author?.login || "")) {
    return { status: "ineligible", reason: "bot author" };
  }
  if (ELIGIBLE_ASSOCIATIONS.has(author?.association)) {
    return { status: "eligible", association: author.association };
  }
  if (!author?.nodeId) {
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
  expectedSha, labels, reviewer, gate, contributor,
  rateLimit, reviewerReason, force, reviewMarkerId,
} = {}) {
  const existing = labelsOf(labels);
  const forced = force === true;
  const fail = (reason, report = false, contributorComment = null) => {
    const comments = [
      forced ? null : quotaComment(rateLimit),
      evidenceLimitComment(reason),
      contributorComment,
    ].filter(Boolean);
    return {
      ok: true, mode: "review", expectedSha, failed: true, reason,
      labelSets: [], addLabels: ["maintainer-required"], comments,
      removeCommentMarkers: [
        ...(comments.some((comment) => comment.kind === "evidence-limit") ? [] : [EVIDENCE_LIMIT_MARKER]),
        FORK_QUOTA_MARKER,
        ...(comments.some((comment) => comment.kind === "global-quota") ? [] : [GLOBAL_QUOTA_MARKER]),
        ...(forced || contributor?.status === "eligible" ? [CONTRIBUTOR_INELIGIBLE_MARKER] : []),
      ],
      ...(report ? { check: {
        name: "AI automated review", externalId: expectedSha,
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
    if (!gate || gate.head_sha !== expectedSha || typeof gate.ok !== "boolean" || gate.reason) {
      const reason = gate?.reason ? `review gate unavailable: ${gate.reason}` : "review gate unavailable";
      return fail(reason);
    }
    if (gate.classificationCheck !== true || gate.ciGreen !== true) return fail("review gate unavailable");
    const route = validateReviewerRoute({
      reviewers: gate.specialistReviewers,
      protocolRelated: gate.protocolRelated,
      risk: gate.risk,
    });
    if (!route.ok) return fail("reviewer route unavailable");
    if (contributor?.status === "ineligible") {
      const reason = Number.isSafeInteger(contributor.merged)
        ? `contributor history ineligible (merged: ${contributor.merged}, required: ${ELIGIBLE_MERGED_PRS})`
        : `contributor history ineligible${contributor.reason ? `: ${contributor.reason}` : ""}`;
      const comment = Number.isSafeInteger(contributor.merged)
        ? { kind: "contributor-ineligible", marker: CONTRIBUTOR_INELIGIBLE_MARKER }
        : null;
      return fail(reason, false, comment);
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
      labels, legitimacyStopped: gate.legitimacyStopped,
    })) return fail("review is not eligible");
    if (!gate.ok) return fail("review gate unavailable");
  }
  const reviewerResult = validateNormalizedFinalReview(reviewer, expectedSha);
  if (!reviewerResult?.ok || reviewerResult.value?.head_sha !== expectedSha) {
    return fail(reviewerReason || reviewerResult?.reason || "reviewer unavailable", true);
  }
  const nextCount = existing.has("ai-reviewed/2") ? "ai-reviewed/2"
    : existing.has("ai-reviewed/1") ? "ai-reviewed/2"
    : "ai-reviewed/1";
  const expectedReviewCount = existing.has("ai-reviewed/2") ? "ai-reviewed/2"
    : existing.has("ai-reviewed/1") ? "ai-reviewed/1"
    : null;
  const hasFindings = reviewerResult.value.findings.length > 0;
  const reviewMarker = `<!-- ironrdp-pr-automation:review:${expectedSha}` +
    `${forced ? `:force:${reviewMarkerId}` : ""} -->`;
  return {
    ok: true, mode: "review", expectedSha, labelSets: [{ owned: AI_COUNTS, desired: [nextCount] }],
    addLabels: nextCount === "ai-reviewed/2" || !hasFindings ? ["maintainer-required"] : [],
    removeLabels: nextCount === "ai-reviewed/1" && hasFindings ? ["maintainer-required"] : [],
    comments: [{ kind: "review", marker: reviewMarker, review: reviewerResult.value }],
    removeCommentMarkers: [
      EVIDENCE_LIMIT_MARKER, FORK_QUOTA_MARKER, GLOBAL_QUOTA_MARKER,
      CONTRIBUTOR_INELIGIBLE_MARKER,
    ],
    check: { name: "AI automated review", externalId: expectedSha },
    expectedReviewCount,
    forced,
    protocolRelated: gate.protocolRelated === true,
  };
}

module.exports = {
  AI_COUNTS, CONTRIBUTOR_INELIGIBLE_MARKER, DUPLICATE_MARKER, EVIDENCE_LIMIT_MARKER,
  FORK_QUOTA_MARKER, GLOBAL_QUOTA_MARKER, LEGACY_XL_MARKER, LEGITIMACY_LABEL,
  LEGITIMACY_MARKER_PREFIX, OVERSIZED_REVIEW_LABEL, RISK, OVERSIZED_MARKER, ELIGIBLE_MERGED_PRS,
  contributorEligibility, qualifyingMergedPrs, resolveClassificationState,
  resolveReviewState, reviewPolicyEligible,
};
