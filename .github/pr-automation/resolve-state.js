"use strict";

const { SCHEMA_VERSION: CLASSIFIER_SCHEMA_VERSION, validateClassifier } = require("./validate-classifier");
const { generation } = require("./ci-state");
const { validateNormalizedFinalReview } = require("./validate-final-review");
const { resolveReviewerRoute, reviewPolicyEligible } = require("./routing");
const { validateReviewGate } = require("./review-pipeline");

const RISK = ["risk/low", "risk/medium", "risk/high", "risk/unknown"];
const AI_COUNTS = ["ai-reviewed/1", "ai-reviewed/2"];
const ACTOR_LABELS = ["needs-review", "needs-author-action"];
const FAILURE_LABEL = "automation-failed";
const LEGITIMACY_LABEL = "triage/legitimacy";
const OVERLAP_LABEL = "triage/overlap";
const LEGITIMACY_MARKER_PREFIX = "<!-- ironrdp-pr-automation:legitimacy:v2:";
const OVERLAP_MARKER = "<!-- ironrdp-pr-automation:overlap -->";
const LEGACY_XL_MARKER = "<!-- ironrdp-pr-automation:xl -->";
const FORK_QUOTA_MARKER = "<!-- ironrdp-pr-automation:fork-llm-quota -->";
const GLOBAL_QUOTA_MARKER = "<!-- ironrdp-pr-automation:fork-llm-global-budget -->";
const EVIDENCE_LIMIT_MARKER = "<!-- ironrdp-pr-automation:evidence-limit -->";
const EVIDENCE_LIMIT_REASON = /^pull request diff exceeds the 4 MiB evidence limit$/;

function labelsOf(labels) {
  return new Set((labels || []).map((label) => typeof label === "string" ? label : label?.name).filter(Boolean));
}

function reviewCount(labels) {
  const present = labels instanceof Set ? labels : labelsOf(labels);
  const first = present.has("ai-reviewed/1");
  const second = present.has("ai-reviewed/2");
  if (first && second) return undefined;
  return second ? "ai-reviewed/2" : first ? "ai-reviewed/1" : null;
}

function failureLabelSets(failure = []) {
  return [{ owned: [FAILURE_LABEL], desired: failure }];
}

function preservedFailureLabelSets(labels) {
  return failureLabelSets(labels.has(FAILURE_LABEL) ? [FAILURE_LABEL] : []);
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
  return typeof reason === "string" && EVIDENCE_LIMIT_REASON.test(reason)
    ? { kind: "evidence-limit", marker: EVIDENCE_LIMIT_MARKER }
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

function failedClassification(expectedSha, deterministic, reason, rateLimit, semverStatus, forced) {
  const comments = [quotaComment(rateLimit), evidenceLimitComment(reason)].filter(Boolean);
  return {
    ok: true, mode: "classification", expectedSha, forced, failed: true, reason,
    labelSets: [
      ...deterministicLabelSets(deterministic),
      { owned: RISK, desired: [semverStatus === "suspected" ? "risk/high" : "risk/unknown"] },
      ...failureLabelSets([FAILURE_LABEL]),
      ...(semverStatus === "suspected"
        ? [{ owned: ["breaking-change"], desired: ["breaking-change"] }]
        : []),
    ],
    comments,
    removeCommentMarkers: [
      ...(comments.some((comment) => comment.kind === "evidence-limit") ? [] : [EVIDENCE_LIMIT_MARKER]),
      FORK_QUOTA_MARKER,
      ...(comments.some((comment) => comment.kind === "global-quota") ? [] : [GLOBAL_QUOTA_MARKER]),
    ],
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: "Classification unavailable",
      summary: `Automated classification was unavailable: ${reason}. Automation remains blocked until retry or repair.`,
      machineState: classificationMachineState({
        risk: semverStatus === "suspected" ? "high" : "unknown",
      }),
      conclusion: "neutral",
    },
  };
}

function resolveClassificationState({
  expectedSha, labels, deterministic, classifier, classificationGate,
  classifierReason, changedPaths, overlapCandidates, prNumber, semver, rateLimit, force,
} = {}) {
  const existing = labelsOf(labels);
  const forced = force === true;
  const failureRateLimit = forced ? undefined : rateLimit;
  if (typeof expectedSha !== "string") return { ok: false, reason: "missing expected SHA" };
  const semverStatus = boundStatus(semver, expectedSha, ["suspected", "not-suspected"]);
  if (!forced && rateLimit && rateLimit.status !== "allowed") {
    return failedClassification(
      expectedSha, deterministic, "fork LLM quota unavailable", failureRateLimit, semverStatus, forced);
  }
  if (!deterministic?.ok) {
    const reason = deterministic?.reason || "deterministic analysis unavailable";
    return failedClassification(expectedSha, deterministic, reason, failureRateLimit, semverStatus, forced);
  }
  if (!forced && classificationGate?.available === false) {
    const reason = classificationGate.reason || "classification gate unavailable";
    return failedClassification(expectedSha, deterministic, reason, failureRateLimit, semverStatus, forced);
  }
  const classifierResult = validateClassifier(classifier, {
    expectedSha, changedPaths, documentationOnlyPaths: deterministic.documentationOnlyPaths,
    overlapCandidates, prNumber,
  });
  if (!classifierResult?.ok || classifierResult.value?.head_sha !== expectedSha) {
    const reason = classifierReason || classifierResult?.reason || "classifier output unavailable";
    return failedClassification(expectedSha, deterministic, reason, failureRateLimit, semverStatus, forced);
  }
  if (semverStatus === "unavailable") {
    return failedClassification(
      expectedSha, deterministic, "public API compatibility unavailable", failureRateLimit, semverStatus, forced);
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
      expectedSha, deterministic, "reviewer routing unavailable", failureRateLimit, semverStatus, forced);
  }
  // Overlap is advisory: it only adds a label and comment.
  const overlap = model.overlap.detected && model.overlap.confidence >= 0.85;
  const optional = [
    ["kind/technical-debt", model.technical_debt],
    ["kind/protocol", model.protocol_related],
    ["documentation", model.documentation_only],
    [OVERLAP_LABEL, overlap],
  ];
  const labelSets = [
    { owned: RISK, desired: [`risk/${risk}`] },
    ...deterministicLabelSets(deterministic),
    { owned: ["scope/cross-cutting"], desired: model.cross_cutting ? ["scope/cross-cutting"] : [] },
    ...optional.map(([label, enabled]) => ({ owned: [label], desired: enabled ? [label] : [] })),
    { owned: ["breaking-change"], desired: breaking ? ["breaking-change"] : [] },
    ...(forced ? preservedFailureLabelSets(existing) : failureLabelSets()),
  ];
  const legitimacyStopped = model.likely_non_legitimate;
  const addLabels = [
    ...(legitimacyStopped ? [LEGITIMACY_LABEL] : []),
  ];
  const comments = [
    ...(overlap ? [{
      kind: "overlap", marker: OVERLAP_MARKER,
      url: model.overlap.similar_pr_url, rationale: model.overlap.rationale,
    }] : []),
  ];
  const auditComments = [
    ...(legitimacyStopped ? [{
      kind: "legitimacy", marker: `${LEGITIMACY_MARKER_PREFIX}${expectedSha} -->`,
      sha: expectedSha, reason: model.non_legitimate_reason,
    }] : []),
  ];
  return {
    ok: true, mode: "classification", expectedSha, forced, labelSets, addLabels, comments, auditComments,
    dispatchReview: !forced,
    removeCommentMarkers: [
      // Remove notices that contradict the current classification.
      ...(overlap ? [] : [OVERLAP_MARKER]),
      EVIDENCE_LIMIT_MARKER,
      FORK_QUOTA_MARKER,
      GLOBAL_QUOTA_MARKER,
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

// Every non-bot author is eligible for automatic review immediately; there is no merged-PR
// history requirement. `github`/`owner`/`repo`/`currentPrNumber` are accepted for call-site
// compatibility but no history lookup is performed. Author identity is required to distinguish a
// bot from a human at all: a missing/null `user` (both `login` and `type` absent) cannot be
// classified, so it reports unavailable rather than eligible.
async function contributorEligibility({ author } = {}) {
  if (typeof author?.login !== "string" && typeof author?.type !== "string") {
    return { status: "unavailable", reason: "missing author identity" };
  }
  if (author.type === "Bot" || /\[bot\]$/i.test(author.login || "")) {
    return { status: "bot" };
  }
  return { status: "eligible", association: author.association ?? null };
}

function resolveReviewState({
  expectedSha, labels, reviewer, gate, contributor,
  rateLimit, reviewerReason, force, reviewMarkerId, reducedCoverage,
  reviewerValidationContext, specialistAggregate, requireReviewerContext = false,
  reviewAttempted = reviewer !== undefined,
} = {}) {
  const existing = labelsOf(labels);
  const forced = force === true;
  const ciGeneration = generation({
    id: gate?.ciRunId,
    run_attempt: gate?.ciRunAttempt,
  });
  const ciState = ciGeneration
    ? { ciRunId: ciGeneration.id, ciRunAttempt: ciGeneration.attempt }
    : {};
  const state = (reason, {
    failed = false, report = false, failure = [],
    blocked = false, labelSets = failureLabelSets(failure),
  } = {}) => {
    const comments = [
      forced ? null : quotaComment(rateLimit),
      evidenceLimitComment(reason),
    ].filter(Boolean);
    return {
      ok: true, mode: "review", expectedSha, forced,
      ...(failed ? { failed: true } : {}),
      ...(blocked ? { blocked: true } : {}),
      reason, labelSets,
      comments,
      removeCommentMarkers: [
        ...(comments.some((comment) => comment.kind === "evidence-limit") ? [] : [EVIDENCE_LIMIT_MARKER]),
        FORK_QUOTA_MARKER,
        ...(comments.some((comment) => comment.kind === "global-quota") ? [] : [GLOBAL_QUOTA_MARKER]),
      ],
      ...(report ? { check: {
        name: "AI automated review", externalId: expectedSha,
        title: "Automated review unavailable",
        summary: `Automated review was unavailable: ${reason}. Automation remains blocked until retry or repair.`,
        conclusion: "neutral",
      } } : {}),
    };
  };
  const blocked = (reason) => state(reason, { blocked: true, labelSets: [] });
  const fail = (reason) => state(reason, {
    failed: true, report: true, failure: [FAILURE_LABEL],
  });
  if (typeof expectedSha !== "string") return { ok: false, reason: "missing expected SHA" };
  const currentReviewCount = reviewCount(existing);
  if (currentReviewCount === undefined) return blocked("review count is ambiguous");
  if (forced) {
    if (gate?.force !== true || gate.head_sha !== expectedSha) return fail("forced review gate unavailable");
    const classification = validateReviewGate(gate, expectedSha);
    if (!classification.ok) return fail(classification.reason);
    if (!Number.isSafeInteger(gate.classificationId) || gate.classificationId <= 0) {
      return fail("forced classification identity unavailable");
    }
    if (typeof reviewMarkerId !== "string" || !/^[1-9]\d{0,19}$/.test(reviewMarkerId)) {
      return fail("forced review marker unavailable");
    }
  } else {
    if (!gate || gate.head_sha !== expectedSha) return blocked("review gate unavailable");
    if (gate.ciGreen !== true) return blocked("CI has not succeeded");
    if (gate.classificationValid === true &&
        (existing.has("ai-reviewed/2") || gate.legitimacyStopped === true ||
         existing.has(LEGITIMACY_LABEL))) {
      return {
        ...state("review is handed to a human"),
        handoff: existing.has("ai-reviewed/2") ? "terminal" : "legitimacy",
        ...ciState,
      };
    }
    if (rateLimit && rateLimit.status !== "allowed") return blocked("fork LLM quota unavailable");
    if (typeof gate.ok !== "boolean" || gate.reason) {
      const reason = gate?.reason ? `review gate unavailable: ${gate.reason}` : "review gate unavailable";
      return blocked(reason);
    }
    const classification = validateReviewGate({ ...gate, ok: true }, expectedSha);
    if (!classification.ok) return blocked(classification.reason);
    if (gate.classificationCheck !== true) return blocked("review gate unavailable");
    if (!reviewPolicyEligible({
      labels, legitimacyStopped: gate.legitimacyStopped,
    })) return blocked("review is not eligible");
    if (contributor?.status === "bot") {
      return blocked("author is a bot account");
    }
    if (contributor?.status !== "eligible") {
      const reason = contributor?.reason
        ? `contributor eligibility unavailable: ${contributor.reason}`
        : "contributor eligibility unavailable";
      return blocked(reason);
    }
    if (gate.reviewAtHead === true ||
        (existing.has("ai-reviewed/1") && gate.secondReviewEligible !== true)) {
      return blocked("an automated review already exists for this head");
    }
    if (!gate.ok) return blocked("review gate unavailable");
    if (!reviewAttempted) return blocked("review was not attempted");
  }
  const reviewerContextMatches = reviewerValidationContext?.head_sha === expectedSha;
  const reviewerResult = validateNormalizedFinalReview(reviewer, {
    expectedSha,
    changedPaths: reviewerContextMatches ? reviewerValidationContext.changed_paths : undefined,
    changedLines: reviewerContextMatches ? reviewerValidationContext.changed_lines : undefined,
    specialistAggregate,
    requireContext: requireReviewerContext,
  });
  if (!reviewerResult?.ok || reviewerResult.value?.head_sha !== expectedSha) {
    return fail(reviewerReason || reviewerResult?.reason || "reviewer unavailable");
  }
  const nextCount = currentReviewCount === "ai-reviewed/2" ? "ai-reviewed/2"
    : currentReviewCount === "ai-reviewed/1" ? "ai-reviewed/2"
    : "ai-reviewed/1";
  const expectedReviewCount = currentReviewCount;
  const hasFindings = reviewerResult.value.findings.length > 0;
  const reviewMarker = `<!-- ironrdp-pr-automation:review:${expectedSha}` +
    `${forced ? `:force:${reviewMarkerId}` : ""} -->`;
  return {
    ok: true, mode: "review", expectedSha,
    labelSets: [
      { owned: AI_COUNTS, desired: [nextCount] },
      ...failureLabelSets(),
    ],
    comments: [{
      kind: "review", marker: reviewMarker, review: reviewerResult.value,
      reducedCoverage: Array.isArray(reducedCoverage) ? reducedCoverage : [],
    }],
    removeCommentMarkers: [EVIDENCE_LIMIT_MARKER, FORK_QUOTA_MARKER, GLOBAL_QUOTA_MARKER],
    check: {
      name: "AI automated review",
      externalId: expectedSha,
      outcome: hasFindings ? "findings" : "no-findings",
    },
    outcome: hasFindings ? "findings" : "no-findings",
    expectedReviewCount,
    nextReviewCount: nextCount,
    forced,
    admittedGate: {
      classificationId: gate.classificationId,
      ciRunId: ciGeneration?.id ?? null,
      ciRunAttempt: ciGeneration?.attempt ?? null,
      labels: gate.labels || [],
      policyEligible: gate.policyEligible,
      legitimacyStopped: gate.legitimacyStopped,
    },
    ...(!forced ? ciState : {}),
    protocolRelated: gate.protocolRelated === true,
  };
}

function reviewOutcome({ reportStatus, state, reducedCoverage = [] } = {}) {
  if (reportStatus !== "success" || state?.failed === true) return "unavailable";
  if (Array.isArray(reducedCoverage) && reducedCoverage.length > 0) {
    return "reduced-coverage";
  }
  return "complete";
}

module.exports = {
  ACTOR_LABELS, AI_COUNTS, EVIDENCE_LIMIT_MARKER, FAILURE_LABEL, FORK_QUOTA_MARKER,
  GLOBAL_QUOTA_MARKER, LEGACY_XL_MARKER, LEGITIMACY_LABEL,
  LEGITIMACY_MARKER_PREFIX, OVERLAP_LABEL, OVERLAP_MARKER, RISK, labelsOf,
  contributorEligibility, resolveClassificationState,
  resolveReviewState, reviewCount, reviewOutcome, reviewPolicyEligible,
};
