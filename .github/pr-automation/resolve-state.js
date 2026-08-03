"use strict";

const { SCHEMA_VERSION: CLASSIFIER_SCHEMA_VERSION } = require("./validate-classifier");
const { SCHEMA_VERSION: REVIEWER_SCHEMA_VERSION } = require("./validate-reviewer");
const { validateClassifier } = require("./validate-classifier");
const { validateReviewer } = require("./validate-reviewer");

const RISK = ["risk:low", "risk:medium", "risk:high"];
const AI_COUNTS = ["ai-reviewed/1", "ai-reviewed/2"];
const LEGITIMACY_MARKER = "<!-- ironrdp-pr-automation:legitimacy:v1 -->";
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
  return !(present.has("risk:low") && !present.has("breaking-change"));
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

function failedClassification(expectedSha, existing, reason, rateLimit) {
  const labels = labelsOf(existing);
  const presentRisk = RISK.filter((label) => labels.has(label));
  const comment = quotaComment(rateLimit);
  return {
    ok: true, mode: "classification", expectedSha, failed: true, reason,
    labelSets: presentRisk.length ? [] : [{ owned: RISK, desired: ["risk:high"] }],
    addLabels: ["human-required"], comments: comment ? [comment] : [],
  };
}

// Bot-authored pull requests get path and size labels and nothing else: no LLM stage runs, so no
// risk label is produced and the review route never opens. The check keeps re-runs at the same head
// idempotent, and its title is deliberately not "Classification complete" so the review gate
// refuses it.
function botClassification(expectedSha, deterministic) {
  if (!deterministic?.ok) {
    return {
      ok: true, mode: "classification", expectedSha, failed: true, reason: "deterministic analysis unavailable",
      labelSets: [], addLabels: ["human-required"], comments: [],
    };
  }
  return {
    ok: true, mode: "classification", expectedSha, botAuthor: true,
    labelSets: [
      { owned: deterministic.ownedPathLabels || [], desired: deterministic.pathLabels || [] },
      { owned: deterministic.sizeLabels || [], desired: [deterministic.sizeLabel].filter(Boolean) },
    ],
    addLabels: ["human-required"], comments: [],
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: "Deterministic labelling only",
      summary: "This pull request was opened by a bot, so no model was invoked.",
      machineState: { protocolRelated: false },
    },
  };
}

function resolveClassificationState({
  expectedSha, labels, deterministic, classifier, changedPaths, prNumber, semver, sspi, rateLimit,
  authorIsBot,
} = {}) {
  const existing = labelsOf(labels);
  if (typeof expectedSha !== "string") return { ok: false, reason: "missing expected SHA" };
  if (existing.has("ai-reviewed/2")) {
    return { ok: true, mode: "classification", expectedSha, terminal: true, labelSets: [],
      addLabels: ["human-required"], comments: [] };
  }
  if (authorIsBot) return botClassification(expectedSha, deterministic);
  const semverStatus = boundStatus(semver, expectedSha, ["suspected", "not-suspected"]);
  const sspiStatus = boundStatus(sspi, expectedSha, ["required", "not-required"]);
  const classifierResult = validateClassifier(classifier, {
    expectedSha, changedPaths, documentationOnlyPaths: deterministic?.documentationOnlyPaths, prNumber,
  });
  if (rateLimit && rateLimit.status !== "allowed") {
    return failedClassification(expectedSha, labels, "fork LLM quota unavailable", rateLimit);
  }
  if (!deterministic?.ok || !classifierResult?.ok || classifierResult.value?.head_sha !== expectedSha ||
      semverStatus === "unavailable" || sspiStatus === "unavailable") {
    return failedClassification(expectedSha, labels, "classification prerequisite unavailable", rateLimit);
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
    ["A-technical-debt", model.technical_debt],
    ["documentation", model.documentation_only],
    ["duplicate", duplicate],
    ["A-sspi", sspiStatus === "required"],
    ["contributor/first-time", deterministic.firstTime],
  ];
  const labelSets = [
    { owned: RISK, desired: [`risk:${risk}`] },
    { owned: deterministic.ownedPathLabels || [], desired: deterministic.pathLabels || [] },
    { owned: deterministic.sizeLabels || [], desired: [deterministic.sizeLabel].filter(Boolean) },
    ...optional.map(([label, enabled]) => ({ owned: [label], desired: enabled ? [label] : [] })),
    { owned: ["breaking-change"], desired: breaking ? ["breaking-change"] : [] },
  ];
  const addLabels = ["human-required"];
  const legitimacyStopped = model.likely_non_legitimate;
  const comments = [
    ...(duplicate ? [{
      kind: "duplicate", marker: "<!-- ironrdp-pr-automation:duplicate -->",
      url: model.duplicate.similar_pr_url, rationale: model.duplicate.rationale,
    }] : []),
    ...(isXl ? [{ kind: "xl", marker: XL_MARKER }] : []),
    ...(legitimacyStopped ? [{
      kind: "legitimacy", marker: LEGITIMACY_MARKER, reason: model.non_legitimate_reason,
    }] : []),
  ];
  return {
    ok: true, mode: "classification", expectedSha, labelSets, addLabels, comments,
    removeCommentMarkers: [
      ...(legitimacyStopped ? [] : [LEGITIMACY_MARKER]),
      // A later push can drop the change below the XL threshold, and stale split guidance would
      // then contradict the labels.
      ...(isXl ? [] : [XL_MARKER]),
    ],
    legitimacyStopped,
    check: {
      name: "AI classification",
      externalId: `${CLASSIFIER_SCHEMA_VERSION}:${expectedSha}`,
      title: legitimacyStopped ? "Automation stopped" : "Classification complete",
      summary: legitimacyStopped
        ? "Validated human-triage classification is bound to this commit."
        : "Validated AI classification is bound to this commit.",
      machineState: { protocolRelated: model.protocol_related },
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
  rateLimit, protocolStatus,
} = {}) {
  const existing = labelsOf(labels);
  const fail = (reason) => {
    const comment = quotaComment(rateLimit);
    return {
      ok: true, mode: "review", expectedSha, failed: true, reason,
      labelSets: [], addLabels: ["human-required"], comments: comment ? [comment] : [],
    };
  };
  if (typeof expectedSha !== "string") return { ok: false, reason: "missing expected SHA" };
  if (existing.has("ai-reviewed/2")) return fail("terminal AI review count");
  if (rateLimit && rateLimit.status !== "allowed") return fail("fork LLM quota unavailable");
  if (!gate?.ok || gate.head_sha !== expectedSha || gate.classificationCheck !== true ||
      (gate.ciGreen !== true && gate.bypassCi !== true) || contributor?.status !== "eligible") {
    return fail("review gate unavailable");
  }
  if (existing.has("ai-reviewed/1") && gate.secondReviewEligible !== true) return fail("second review is not eligible");
  if (!reviewPolicyEligible({
    labels, legitimacyStopped: gate.legitimacyStopped, protocolRelated: gate.protocolRelated,
  })) return fail("review is not eligible");
  // A protocol-related review is only publishable when the protocol stage produced a validated
  // handoff; anything else fails closed to humans.
  if (!["valid", "not_applicable"].includes(protocolStatus)) return fail("protocol handoff unavailable");
  const reviewerResult = validateReviewer(reviewer, {
    expectedSha, changedPaths, changedLines, protocolReceived: protocolStatus === "valid",
  });
  if (!reviewerResult?.ok || reviewerResult.value?.head_sha !== expectedSha) return fail("reviewer unavailable");
  const nextCount = existing.has("ai-reviewed/1") ? "ai-reviewed/2" : "ai-reviewed/1";
  const hasFindings = reviewerResult.value.has_findings;
  return {
    ok: true, mode: "review", expectedSha, labelSets: [{ owned: AI_COUNTS, desired: [nextCount] }],
    addLabels: nextCount === "ai-reviewed/2" || !hasFindings ? ["human-required"] : [],
    removeLabels: nextCount === "ai-reviewed/1" && hasFindings ? ["human-required"] : [],
    comments: hasFindings ? [{ kind: "review", marker: `<!-- ironrdp-pr-automation:review:${expectedSha} -->`,
      review: reviewerResult.value }] : [],
    check: { name: "AI automated review", externalId: `${REVIEWER_SCHEMA_VERSION}:${expectedSha}` },
    reviewerSchemaVersion: REVIEWER_SCHEMA_VERSION,
  };
}

module.exports = {
  AI_COUNTS, FORK_QUOTA_MARKER, GLOBAL_QUOTA_MARKER, LEGITIMACY_MARKER, RISK, XL_MARKER,
  ELIGIBLE_MERGED_PRS, contributorEligibility, isExcludedHistory, qualifyingMergedPrs,
  resolveClassificationState, resolveReviewState, reviewPolicyEligible,
};
