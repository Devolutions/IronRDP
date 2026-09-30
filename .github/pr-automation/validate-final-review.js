"use strict";

const {
  MAXIMUM_GITHUB_INTEGER, REPO_PATH, SHA, exactKeys, invalid, isBoundedArray, linesAreInRange,
  linesAreValidated, normalizeText, parseJson,
  unicodeLength,
} = require("./validation");
const {
  MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS, inlineReviewCommentBody, reviewBody,
} = require("./review-render");
const { REVIEWER_ORDER: REVIEWERS } = require("./routing");

const MAXIMUM_CANDIDATES = 60;
const MAXIMUM_FINDINGS = 20;
const MAXIMUM_SUMMARY_LENGTH = 1000;
const MAXIMUM_DISPOSITION_RATIONALE_LENGTH = 800;
const MAXIMUM_TITLE_LENGTH = 200;
const MAXIMUM_RATIONALE_LENGTH = 1200;
const MAXIMUM_PATH_LENGTH = 300;
const SEVERITIES = new Set(["critical", "high", "medium", "low"]);
const DISPOSITIONS = new Set(["accepted", "refined", "rejected"]);
const FINDING_ID = /^[a-z][a-z0-9-]{0,63}$/;
const REVIEWER_ORDER = new Map(REVIEWERS.map((reviewer, index) => [reviewer, index]));
const MAXIMUM_REVIEW_MARKER =
  `<!-- ironrdp-pr-automation:review:${"f".repeat(40)}:force:${"9".repeat(20)} -->`;

// A rejection is both repair feedback and a published stage reason, and the runtime is the tighter
// of the two consumers: `sanitizeReason` keeps 240 bytes of the reason for its diagnostics, and the
// same 240 bytes of `semantic: <reason>` for the exhaustion failure. A reason within the smaller of
// those allowances survives every path whole, which is what this budget is, and it is comfortably
// inside the 300 bytes the stage report keeps.
const RUNTIME_REASON_BYTES = 240;
const MAXIMUM_REASON_BYTES = RUNTIME_REASON_BYTES - "semantic: ".length;
const MAXIMUM_COORDINATES = 8;
// The reason is what survives exhaustion, so it stays that small. The runtime also accepts a longer
// detail, which it sends as repair feedback and keeps per attempt, and repair-only guidance, which
// it never logs. These mirror its `MAX_VALIDATION_DETAIL_BYTES` and `MAX_VALIDATION_GUIDANCE_BYTES`.
const MAXIMUM_DETAIL_BYTES = 2048;
const MAXIMUM_GUIDANCE_BYTES = 16 * 1024;

const count = (value, noun) => `${value} ${noun}${value === 1 ? "" : "s"}`;

// `normalizeText` checks character limits and forbidden controls; callers also reject empty results.
const NORMALIZED_TEXT_RULE = "non-blank and free of forbidden control characters";

function referenceKey(reference) {
  return `${reference.reviewer}\0${reference.finding_id}`;
}

function normalizeReference(value) {
  if (!exactKeys(value, ["reviewer", "finding_id"]) ||
      !REVIEWER_ORDER.has(value.reviewer) || !FINDING_ID.test(value.finding_id)) return null;
  return { reviewer: value.reviewer, finding_id: value.finding_id };
}

function sourceCategories(sources) {
  if (sources.length === 0) return ["general"];
  if (sources.some((source) => normalizeReference(source) === null)) {
    throw new Error("invalid validated finding source");
  }
  return [...new Set(sources.map((source) => source.reviewer))]
    .sort((left, right) => REVIEWER_ORDER.get(left) - REVIEWER_ORDER.get(right));
}

function provenancePrefix(sources) {
  return `[${sourceCategories(sources).join(" + ")}]`;
}

function publicationFits(review) {
  if (reviewBody(MAXIMUM_REVIEW_MARKER, review, REVIEWERS, provenancePrefix).length >
      MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS) return false;
  return review.findings
    .filter((finding) => finding.start_line !== null)
    .every((finding) =>
      inlineReviewCommentBody(finding, provenancePrefix).length <=
        MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS);
}

// A repair has to find the candidate a diagnostic is about without the diagnostic quoting anything
// the model wrote, so a candidate is named by its reviewer and its position in that reviewer's
// findings inside the trusted aggregate. Reviewer names come from the pipeline's own enum.
function boundedCoordinates(items, format, limit) {
  const shown = items.slice(0, limit);
  const omitted = items.length - shown.length;
  return omitted === 0 ? format(shown) : `${format(shown)} and ${omitted} more`;
}

function aggregateOrder(candidates) {
  return [...candidates].sort((left, right) =>
    REVIEWER_ORDER.get(left.reviewer) - REVIEWER_ORDER.get(right.reviewer) || left.index - right.index);
}

function aggregateCoordinates(candidates, limit = MAXIMUM_COORDINATES) {
  return boundedCoordinates(aggregateOrder(candidates), (shown) => {
    const byReviewer = new Map();
    for (const candidate of shown) {
      byReviewer.set(candidate.reviewer, [...(byReviewer.get(candidate.reviewer) ?? []), candidate.index]);
    }
    return [...byReviewer]
      .map(([reviewer, indexes]) => `${reviewer} ${indexes.join(", ")}`)
      .join(" and ");
  }, limit);
}

function indexCoordinates(indexes, limit = MAXIMUM_COORDINATES) {
  return boundedCoordinates(indexes, (shown) => shown.join(", "), limit);
}

// A repair cannot copy an identifier it was never shown, and a reviewer that spent its turns before
// reading the aggregate has none, so positions alone let it invent plausible ones. The identifiers
// quoted here are the trusted aggregate's, which admitted only `FINDING_ID` spellings, and guidance
// only ever reaches the repair conversation.
function candidateGuidance(lead, candidates) {
  if (candidates.length === 0) return undefined;
  const listed = aggregateOrder(candidates)
    .map((candidate) => `(${candidate.reviewer}, ${candidate.finding_id})`)
    .join("; ");
  const guidance = `${lead}, as (reviewer, finding_id), copied exactly: ${listed}`;
  return Buffer.byteLength(guidance, "utf8") <= MAXIMUM_GUIDANCE_BYTES ? guidance : undefined;
}

// The counts and categories say how much of what is wrong, so they are what a repair cannot do
// without; the coordinates only save it a search. Detail is therefore dropped from the end until the
// whole diagnostic fits, and a saturated review falls back to terse forms that still name every
// category and count. Nothing dropped is ever silently forgotten, and the result always fits whole
// inside the runtime's allowance rather than relying on where a truncation happens to land.
function boundedReason(prefix, parts) {
  const fits = (reason) => Buffer.byteLength(reason, "utf8") <= MAXIMUM_REASON_BYTES;
  const assemble = (render) => `${prefix}: ${parts.map(render).join(". ")}`;
  for (let detailed = parts.length; detailed > 0; detailed -= 1) {
    const reason = assemble((part, index) => (index < detailed
      ? `${part.summary}, at ${part.detail(MAXIMUM_COORDINATES)}`
      : part.summary));
    if (fits(reason)) return reason;
  }
  const counted = assemble((part) => part.summary);
  if (fits(counted)) return counted;
  // A review that saturates every failure class at once leaves no room for the sentences that
  // explain them, so what survives is the count and constraint of each one.
  const terse = assemble((part) => part.short ?? part.summary);
  return fits(terse) ? terse : prefix;
}

// What the reason had to drop still reaches repair and the per-attempt diagnostics as the detail:
// every coordinate when that fits the runtime's allowance, and the bounded coordinates otherwise.
// A detail that adds nothing to the reason is left out.
function diagnosis(prefix, parts, guidance) {
  const reason = boundedReason(prefix, parts);
  const detail = [Number.POSITIVE_INFINITY, MAXIMUM_COORDINATES]
    .map((limit) => `${prefix}: ${parts.map((part) => `${part.summary}, at ${part.detail(limit)}`).join(". ")}`)
    .find((candidate) => Buffer.byteLength(candidate, "utf8") <= MAXIMUM_DETAIL_BYTES);
  return {
    reason,
    ...(detail === undefined || detail === reason ? {} : { detail }),
    ...(guidance === undefined ? {} : { guidance }),
  };
}

function rejected({ reason, detail, guidance }) {
  return {
    ...invalid(reason),
    ...(detail === undefined ? {} : { detail }),
    ...(guidance === undefined ? {} : { guidance }),
  };
}

function aggregateCandidates(specialistAggregate, expectedSha) {
  const aggregate = specialistAggregate;
  if (!exactKeys(aggregate, ["head_sha", "reviewers"]) ||
      aggregate.head_sha !== expectedSha || !SHA.test(aggregate.head_sha) ||
      !isBoundedArray(aggregate.reviewers, REVIEWERS.length)) {
    return null;
  }
  const candidates = new Map();
  let previousReviewer = -1;
  const candidateKeys = [
    "id", "question", "severity", "path", "start_line", "end_line", "title", "rationale",
    "confidence", "references",
  ];
  for (const review of aggregate.reviewers) {
    const reviewerIndex = REVIEWER_ORDER.get(review?.reviewer);
    if (reviewerIndex === undefined || reviewerIndex <= previousReviewer) return null;
    previousReviewer = reviewerIndex;
    if (review.status === "failed") {
      if (!exactKeys(review, ["reviewer", "status", "reason"]) ||
          !normalizeText(review.reason, 300)) return null;
      continue;
    }
    if (review.status !== "valid" ||
        !exactKeys(review, ["reviewer", "status", "summary", "findings"]) ||
        !normalizeText(review.summary, 1000) ||
        !isBoundedArray(review.findings, MAXIMUM_FINDINGS)) return null;
    for (const [index, candidate] of review.findings.entries()) {
      if (!exactKeys(candidate, candidateKeys) ||
          typeof candidate.question !== "boolean" ||
          !SEVERITIES.has(candidate.severity) ||
          !Array.isArray(candidate.references)) return null;
      const reference = normalizeReference({
        reviewer: review.reviewer,
        finding_id: candidate.id,
      });
      if (reference === null || candidates.has(referenceKey(reference))) return null;
      // The position in the aggregate is what a diagnostic can safely name this candidate by.
      candidates.set(referenceKey(reference), { ...reference, index });
    }
  }
  return candidates;
}

// The general reviewer is handed the candidate identifiers up front, so accounting for every
// candidate never depends on it reading the aggregate before its investigation budget runs out.
// Only the identifiers the aggregate admitted are listed; what the specialists wrote stays in the
// file, which the reviewer still reads as untrusted evidence. An unusable aggregate yields nothing
// here, since the validator fails the stage on it anyway.
function candidateIndexPrompt(specialistAggregate, expectedSha) {
  const candidates = aggregateCandidates(specialistAggregate, expectedSha);
  if (candidates === null) return "";
  if (candidates.size === 0) {
    return "The validated specialist aggregate reports no candidates, so `candidate_dispositions` must be empty.";
  }
  return [
    `The validated specialist aggregate reports ${count(candidates.size, "candidate")}. ` +
      "Write exactly one `candidate_dispositions` entry for each, copying `reviewer` and `finding_id` exactly:",
    ...aggregateOrder(candidates.values()).map((candidate) =>
      `- reviewer \`${candidate.reviewer}\`, finding_id \`${candidate.finding_id}\``),
  ].join("\n");
}

// Every disposition problem is reported together. The stage affords two repairs and a review can
// carry sixty candidates, so a diagnostic that revealed one missing disposition per attempt could
// not converge on a review that omitted four.
function diagnoseDispositions(entries, candidates) {
  if (!isBoundedArray(entries, MAXIMUM_CANDIDATES)) {
    return {
      ok: false,
      reason: `invalid specialist candidate dispositions: candidate_dispositions must be an array of at most ${MAXIMUM_CANDIDATES} entries`,
    };
  }
  const malformed = [];
  const unknown = [];
  const duplicated = [];
  const unusableRationale = [];
  const seenCandidates = new Set();
  const byCandidate = new Map();
  for (const [index, entry] of entries.entries()) {
    if (!exactKeys(entry, ["reviewer", "finding_id", "disposition", "rationale"]) ||
        !DISPOSITIONS.has(entry.disposition)) {
      malformed.push(index);
      continue;
    }
    const reference = normalizeReference({
      reviewer: entry.reviewer,
      finding_id: entry.finding_id,
    });
    if (reference === null) {
      malformed.push(index);
      continue;
    }
    const key = referenceKey(reference);
    const known = candidates.has(key);
    if (!known) {
      unknown.push(index);
    } else {
      if (seenCandidates.has(key)) duplicated.push(index);
      seenCandidates.add(key);
    }
    const rationale = normalizeText(entry.rationale, MAXIMUM_DISPOSITION_RATIONALE_LENGTH);
    if (!rationale) {
      unusableRationale.push(index);
    } else if (known && !byCandidate.has(key)) {
      byCandidate.set(key, { ...reference, disposition: entry.disposition, rationale });
    }
  }
  const missing = [...candidates].filter(([key]) => !byCandidate.has(key)).map(([, value]) => value);
  const parts = [];
  if (missing.length !== 0) {
    // An entry that names the candidate but carries no usable rationale leaves it here too, so this
    // asks for a valid disposition rather than for another entry.
    parts.push({
      summary: `${missing.length} of ${count(candidates.size, "candidate")} ${missing.length === 1 ? "has" : "have"} no valid disposition`,
      short: `${missing.length}/${candidates.size} candidates lack a valid disposition`,
      detail: (limit) => `aggregate findings ${aggregateCoordinates(missing, limit)}`,
    });
  }
  for (const [indexes, summary, short] of [
    [unknown, "naming a candidate the specialists did not report", "unknown"],
    [duplicated, "repeating a candidate an earlier entry already covered", "duplicate"],
    [unusableRationale,
      `with a rationale that must be ${NORMALIZED_TEXT_RULE}, within ${MAXIMUM_DISPOSITION_RATIONALE_LENGTH} characters`,
      `with a blank, forbidden-control, or over ${MAXIMUM_DISPOSITION_RATIONALE_LENGTH} character rationale`],
    [malformed, "not well formed for a known reviewer", "malformed"],
  ]) {
    if (indexes.length === 0) continue;
    const entries = `${indexes.length} ${indexes.length === 1 ? "entry" : "entries"}`;
    parts.push({
      summary: `${entries} ${summary}`,
      short: `${indexes.length} ${short}`,
      detail: (limit) => `candidate_dispositions index ${indexCoordinates(indexes, limit)}`,
    });
  }
  if (parts.length === 0) return { ok: true, value: byCandidate };
  return {
    ok: false,
    ...diagnosis("invalid specialist candidate dispositions", parts, candidateGuidance(
      "Each of these candidates still needs exactly one candidate_dispositions entry", missing,
    )),
  };
}

function normalizeFinding(finding, changedPaths, changedLines, dispositions, referencedCandidates) {
  const keys = [
    "question", "severity", "path", "start_line", "end_line", "title", "rationale",
    "confidence", "sources",
  ];
  const rejected = (reason) => ({ ok: false, reason });
  if (!exactKeys(finding, keys) ||
      typeof finding.question !== "boolean" ||
      !SEVERITIES.has(finding.severity) ||
      !Number.isFinite(finding.confidence) || finding.confidence < 0 || finding.confidence > 1) {
    return rejected("it must carry exactly the required fields, a boolean question, a known severity, and a confidence between 0 and 1");
  }
  if (typeof finding.path !== "string" || unicodeLength(finding.path) > MAXIMUM_PATH_LENGTH ||
      finding.path.includes("\\") || !REPO_PATH.test(finding.path) || !changedPaths.has(finding.path)) {
    return rejected(`path must be a repository path this pull request changed, within ${MAXIMUM_PATH_LENGTH} characters`);
  }
  if (!isBoundedArray(finding.sources, MAXIMUM_CANDIDATES)) {
    return rejected(`sources must be an array of at most ${MAXIMUM_CANDIDATES} entries`);
  }

  const lineIsAboveGitHubMaximum = [finding.start_line, finding.end_line].some((line) =>
    Number.isSafeInteger(line) && line > MAXIMUM_GITHUB_INTEGER);
  if (lineIsAboveGitHubMaximum) {
    return rejected(`start_line and end_line must be between 1 and ${MAXIMUM_GITHUB_INTEGER}`);
  }
  const linesAreNull = finding.start_line === null && finding.end_line === null;
  const linesAreIntegers = linesAreInRange(finding.start_line, finding.end_line);
  if (!linesAreNull && !linesAreIntegers) {
    return rejected("start_line and end_line must both be null or integers with end_line at or after start_line");
  }

  const title = normalizeText(finding.title, MAXIMUM_TITLE_LENGTH);
  const rationale = normalizeText(finding.rationale, MAXIMUM_RATIONALE_LENGTH);
  if (!title || !rationale) {
    return rejected(`title and rationale must be ${NORMALIZED_TEXT_RULE}; character limits: title ${MAXIMUM_TITLE_LENGTH}, rationale ${MAXIMUM_RATIONALE_LENGTH}`);
  }

  const sources = [];
  const localSources = new Set();
  for (const [index, source] of finding.sources.entries()) {
    const reference = normalizeReference(source);
    if (reference === null) {
      return rejected(`sources index ${index} must name a reviewer and a finding_id`);
    }
    const key = referenceKey(reference);
    const disposition = dispositions.get(key);
    if (!disposition) {
      return rejected(`sources index ${index} names a candidate the specialists did not report`);
    }
    if (disposition.disposition === "rejected") {
      return rejected(`sources index ${index} names a candidate this review rejected`);
    }
    if (localSources.has(key) || referencedCandidates.has(key)) {
      return rejected(`sources index ${index} names a candidate another source already cites`);
    }
    localSources.add(key);
    referencedCandidates.add(key);
    sources.push(reference);
  }
  sources.sort((left, right) =>
    REVIEWER_ORDER.get(left.reviewer) - REVIEWER_ORDER.get(right.reviewer) ||
    left.finding_id.localeCompare(right.finding_id));

  const locationIsValidated = linesAreNull ||
    linesAreValidated(finding.path, finding.start_line, finding.end_line, changedLines);
  return {
    ok: true,
    value: {
      question: finding.question,
      severity: finding.severity,
      path: finding.path,
      start_line: locationIsValidated ? finding.start_line : null,
      end_line: locationIsValidated ? finding.end_line : null,
      title,
      rationale,
      confidence: finding.confidence,
      sources,
    },
  };
}

function validateFinalReview(raw, {
  expectedSha, changedPaths = [], changedLines = {}, specialistAggregate,
} = {}) {
  const candidates = aggregateCandidates(specialistAggregate, expectedSha);
  if (candidates === null) return invalid("validated specialist findings unavailable");

  const value = parseJson(raw);
  if (!exactKeys(value, ["head_sha", "summary", "candidate_dispositions", "findings"]) ||
      !SHA.test(value.head_sha) || value.head_sha !== expectedSha ||
      !isBoundedArray(value.findings, MAXIMUM_FINDINGS)) {
    return invalid("invalid final review object");
  }
  const summary = normalizeText(value.summary, MAXIMUM_SUMMARY_LENGTH);
  if (!summary) {
    return invalid(`invalid final review summary: summary must be ${NORMALIZED_TEXT_RULE}, within ${MAXIMUM_SUMMARY_LENGTH} characters`);
  }

  const dispositions = diagnoseDispositions(value.candidate_dispositions, candidates);
  if (!dispositions.ok) return rejected(dispositions);
  const normalizedDispositions = dispositions.value;

  const findings = [];
  const referencedCandidates = new Set();
  const paths = new Set(changedPaths);
  for (const [index, finding] of value.findings.entries()) {
    const normalized = normalizeFinding(
      finding, paths, changedLines, normalizedDispositions, referencedCandidates,
    );
    if (!normalized.ok) {
      return invalid(`invalid final review finding at index ${index}: ${normalized.reason}`);
    }
    findings.push(normalized.value);
  }

  const uncited = [];
  for (const [key, disposition] of normalizedDispositions) {
    const referenced = referencedCandidates.has(key);
    if (disposition.disposition === "rejected") {
      if (referenced) {
        return invalid("specialist disposition contradicts final findings: a rejected candidate is cited as a source");
      }
    } else if (!referenced) {
      uncited.push(candidates.get(key));
    }
  }
  if (uncited.length !== 0) {
    return rejected(diagnosis("specialist disposition contradicts final findings", [{
      summary: `${count(uncited.length, "accepted or refined candidate")} ${uncited.length === 1 ? "is" : "are"} cited by no final finding`,
      short: `${uncited.length} accepted or refined candidates are uncited`,
      detail: (limit) => `aggregate findings ${aggregateCoordinates(uncited, limit)}`,
    }], candidateGuidance(
      "Cite each of these accepted or refined candidates in the sources of exactly one final finding",
      uncited,
    )));
  }

  const normalized = {
    head_sha: value.head_sha,
    summary,
    findings,
  };
  if (!publicationFits(normalized)) {
    return invalid(
      "final review exceeds GitHub's review-body limit after Markdown escaping; shorten its summary, titles, paths, or rationales",
    );
  }
  return { ok: true, status: "valid", value: normalized };
}

function validateNormalizedFinalReview(value, expectedShaOrContext) {
  const context = typeof expectedShaOrContext === "string"
    ? { expectedSha: expectedShaOrContext }
    : expectedShaOrContext ?? {};
  const {
    expectedSha, changedPaths, changedLines, specialistAggregate, requireContext = false,
  } = context;
  if (!exactKeys(value, [
    "head_sha", "summary", "findings",
  ]) || value.head_sha !== expectedSha || !SHA.test(value.head_sha) ||
      !isBoundedArray(value.findings, MAXIMUM_FINDINGS)) return invalid("invalid validated final review");
  const summary = normalizeText(value.summary, MAXIMUM_SUMMARY_LENGTH);
  if (!summary || summary !== value.summary) return invalid("invalid validated final review summary");
  const paths = Array.isArray(changedPaths) ? new Set(changedPaths) : null;
  const candidates = specialistAggregate === undefined
    ? null
    : aggregateCandidates(specialistAggregate, expectedSha);
  if (requireContext && (!paths || changedLines === null || typeof changedLines !== "object" ||
      candidates === null)) {
    return invalid("validated final review context unavailable");
  }
  const referencedCandidates = new Set();
  for (const finding of value.findings) {
    if (!exactKeys(finding, [
      "question", "severity", "path", "start_line", "end_line", "title", "rationale",
      "confidence", "sources",
    ]) || typeof finding.question !== "boolean" ||
        !SEVERITIES.has(finding.severity) ||
        typeof finding.path !== "string" || unicodeLength(finding.path) > MAXIMUM_PATH_LENGTH ||
        finding.path.includes("\\") || !REPO_PATH.test(finding.path) ||
        (paths && !paths.has(finding.path)) ||
        !Number.isFinite(finding.confidence) || finding.confidence < 0 || finding.confidence > 1 ||
        !isBoundedArray(finding.sources, MAXIMUM_CANDIDATES)) {
      return invalid("invalid validated final review finding");
    }
    const linesAreNull = finding.start_line === null && finding.end_line === null;
    const linesAreIntegers = linesAreInRange(finding.start_line, finding.end_line);
    if (!linesAreNull && (!linesAreIntegers ||
        (changedLines && !linesAreValidated(
          finding.path, finding.start_line, finding.end_line, changedLines)))) {
      return invalid("invalid validated final review finding lines");
    }
    const title = normalizeText(finding.title, MAXIMUM_TITLE_LENGTH);
    const rationale = normalizeText(finding.rationale, MAXIMUM_RATIONALE_LENGTH);
    if (!title || title !== finding.title || !rationale || rationale !== finding.rationale) {
      return invalid("invalid validated final review finding text");
    }
    const localSources = new Set();
    for (const source of finding.sources) {
      const reference = normalizeReference(source);
      if (reference === null) return invalid("invalid validated final review source");
      const key = referenceKey(reference);
      if (localSources.has(key) || referencedCandidates.has(key) ||
          (candidates && !candidates.has(key))) {
        return invalid("invalid validated final review source");
      }
      localSources.add(key);
      referencedCandidates.add(key);
    }
  }
  if (!publicationFits(value)) return invalid("validated final review exceeds GitHub's review-body limit");
  return { ok: true, status: "valid", value };
}

module.exports = {
  MAXIMUM_CANDIDATES, MAXIMUM_DETAIL_BYTES, MAXIMUM_FINDINGS, MAXIMUM_GUIDANCE_BYTES, REVIEWERS,
  candidateIndexPrompt, provenancePrefix, sourceCategories, validateFinalReview,
  validateNormalizedFinalReview,
};
