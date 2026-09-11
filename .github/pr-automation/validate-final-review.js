"use strict";

const {
  REPO_PATH, SHA, exactKeys, invalid, isBoundedArray, linesAreValidated, normalizeText, parseJson,
} = require("./validation");
const { REVIEWER_ORDER: REVIEWERS } = require("./routing");

const MAXIMUM_BYTES = 65536;
const MAXIMUM_CANDIDATES = 60;
const MAXIMUM_FINDINGS = 20;
const MAXIMUM_SUMMARY_BYTES = 1000;
const MAXIMUM_DISPOSITION_RATIONALE_BYTES = 800;
const MAXIMUM_TITLE_BYTES = 200;
const MAXIMUM_RATIONALE_BYTES = 1200;
const MAXIMUM_PATH_BYTES = 300;
const SEVERITIES = new Set(["critical", "high", "medium", "low"]);
const DISPOSITIONS = new Set(["accepted", "refined", "rejected"]);
const FINDING_ID = /^[a-z][a-z0-9-]{0,63}$/;
const REVIEWER_ORDER = new Map(REVIEWERS.map((reviewer, index) => [reviewer, index]));

// A rejection is both repair feedback and a published stage reason, and the runtime is the tighter
// of the two consumers: `sanitizeReason` keeps 240 bytes of the reason for its diagnostics, and the
// same 240 bytes of `semantic: <reason>` for the exhaustion failure. A reason within the smaller of
// those allowances survives every path whole, which is what this budget is, and it is comfortably
// inside the 300 bytes the stage report keeps.
const RUNTIME_REASON_BYTES = 240;
const MAXIMUM_REASON_BYTES = RUNTIME_REASON_BYTES - "semantic: ".length;
const MAXIMUM_COORDINATES = 8;

const count = (value, noun) => `${value} ${noun}${value === 1 ? "" : "s"}`;

// `normalizeText` collapses whitespace and then rejects an empty result, one over the byte budget,
// and any forbidden control character, so a diagnostic about it has to name all three.
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

// A repair has to find the candidate a diagnostic is about without the diagnostic quoting anything
// the model wrote, so a candidate is named by its reviewer and its position in that reviewer's
// findings inside the trusted aggregate. Reviewer names come from the pipeline's own enum.
function boundedCoordinates(items, format) {
  const shown = items.slice(0, MAXIMUM_COORDINATES);
  const omitted = items.length - shown.length;
  return omitted === 0 ? format(shown) : `${format(shown)} and ${omitted} more`;
}

function aggregateCoordinates(candidates) {
  const ordered = [...candidates].sort((left, right) =>
    REVIEWER_ORDER.get(left.reviewer) - REVIEWER_ORDER.get(right.reviewer) || left.index - right.index);
  return boundedCoordinates(ordered, (shown) => {
    const byReviewer = new Map();
    for (const candidate of shown) {
      byReviewer.set(candidate.reviewer, [...(byReviewer.get(candidate.reviewer) ?? []), candidate.index]);
    }
    return [...byReviewer]
      .map(([reviewer, indexes]) => `${reviewer} ${indexes.join(", ")}`)
      .join(" and ");
  });
}

function indexCoordinates(indexes) {
  return boundedCoordinates(indexes, (shown) => shown.join(", "));
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
    const reason = assemble((part, index) => (index < detailed ? `${part.summary}, at ${part.detail}` : part.summary));
    if (fits(reason)) return reason;
  }
  const counted = assemble((part) => part.summary);
  if (fits(counted)) return counted;
  // A review that saturates every failure class at once leaves no room for the sentences that
  // explain them, so what survives is the count and constraint of each one.
  const terse = assemble((part) => part.short ?? part.summary);
  return fits(terse) ? terse : prefix;
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
    const rationale = normalizeText(entry.rationale, MAXIMUM_DISPOSITION_RATIONALE_BYTES);
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
      detail: `aggregate findings ${aggregateCoordinates(missing)}`,
    });
  }
  for (const [indexes, summary, short] of [
    [unknown, "naming a candidate the specialists did not report", "unknown"],
    [duplicated, "repeating a candidate an earlier entry already covered", "duplicate"],
    [unusableRationale,
      `with a rationale that must be ${NORMALIZED_TEXT_RULE}, within ${MAXIMUM_DISPOSITION_RATIONALE_BYTES} UTF-8 bytes`,
      `with a blank, forbidden-control, or over ${MAXIMUM_DISPOSITION_RATIONALE_BYTES} UTF-8 byte rationale`],
    [malformed, "not well formed for a known reviewer", "malformed"],
  ]) {
    if (indexes.length === 0) continue;
    const entries = `${indexes.length} ${indexes.length === 1 ? "entry" : "entries"}`;
    parts.push({
      summary: `${entries} ${summary}`,
      short: `${indexes.length} ${short}`,
      detail: `candidate_dispositions index ${indexCoordinates(indexes)}`,
    });
  }
  if (parts.length === 0) return { ok: true, value: byCandidate };
  return { ok: false, reason: boundedReason("invalid specialist candidate dispositions", parts) };
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
  if (typeof finding.path !== "string" || Buffer.byteLength(finding.path, "utf8") > MAXIMUM_PATH_BYTES ||
      finding.path.includes("\\") || !REPO_PATH.test(finding.path) || !changedPaths.has(finding.path)) {
    return rejected(`path must be a repository path this pull request changed, within ${MAXIMUM_PATH_BYTES} UTF-8 bytes`);
  }
  if (!isBoundedArray(finding.sources, MAXIMUM_CANDIDATES)) {
    return rejected(`sources must be an array of at most ${MAXIMUM_CANDIDATES} entries`);
  }

  const linesAreNull = finding.start_line === null && finding.end_line === null;
  const linesAreIntegers = Number.isSafeInteger(finding.start_line) && finding.start_line >= 1 &&
    Number.isSafeInteger(finding.end_line) && finding.end_line >= finding.start_line;
  if (!linesAreNull && !linesAreIntegers) {
    return rejected("start_line and end_line must both be null or integers with end_line at or after start_line");
  }

  const title = normalizeText(finding.title, MAXIMUM_TITLE_BYTES);
  const rationale = normalizeText(finding.rationale, MAXIMUM_RATIONALE_BYTES);
  if (!title || !rationale) {
    return rejected(`title and rationale must be ${NORMALIZED_TEXT_RULE}; UTF-8 limits: title ${MAXIMUM_TITLE_BYTES} bytes, rationale ${MAXIMUM_RATIONALE_BYTES} bytes`);
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

  const value = parseJson(raw, MAXIMUM_BYTES);
  if (!exactKeys(value, ["head_sha", "summary", "candidate_dispositions", "findings"]) ||
      !SHA.test(value.head_sha) || value.head_sha !== expectedSha ||
      !isBoundedArray(value.findings, MAXIMUM_FINDINGS)) {
    return invalid("invalid final review object");
  }
  const summary = normalizeText(value.summary, MAXIMUM_SUMMARY_BYTES);
  if (!summary) {
    return invalid(`invalid final review summary: summary must be ${NORMALIZED_TEXT_RULE}, within ${MAXIMUM_SUMMARY_BYTES} UTF-8 bytes`);
  }

  const dispositions = diagnoseDispositions(value.candidate_dispositions, candidates);
  if (!dispositions.ok) return invalid(dispositions.reason);
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
    return invalid(boundedReason("specialist disposition contradicts final findings", [{
      summary: `${count(uncited.length, "accepted or refined candidate")} ${uncited.length === 1 ? "is" : "are"} cited by no final finding`,
      short: `${uncited.length} accepted or refined candidates are uncited`,
      detail: `aggregate findings ${aggregateCoordinates(uncited)}`,
    }]));
  }

  const normalized = {
    head_sha: value.head_sha,
    summary,
    findings,
  };
  if (Buffer.byteLength(JSON.stringify(normalized), "utf8") > MAXIMUM_BYTES) {
    return invalid("final review output too large");
  }
  return { ok: true, status: "valid", value: normalized };
}

function validateNormalizedFinalReview(value, expectedSha) {
  if (!exactKeys(value, [
    "head_sha", "summary", "findings",
  ]) || value.head_sha !== expectedSha || !SHA.test(value.head_sha) ||
      !isBoundedArray(value.findings, MAXIMUM_FINDINGS)) return invalid("invalid validated final review");
  for (const finding of value.findings) {
    if (!exactKeys(finding, [
      "question", "severity", "path", "start_line", "end_line", "title", "rationale",
      "confidence", "sources",
    ]) || typeof finding.question !== "boolean" ||
        !SEVERITIES.has(finding.severity)) return invalid("invalid validated final review finding");
    try {
      provenancePrefix(finding.sources);
    } catch {
      return invalid("invalid validated final review source");
    }
  }
  return { ok: true, status: "valid", value };
}

module.exports = {
  MAXIMUM_CANDIDATES, MAXIMUM_FINDINGS, REVIEWERS, provenancePrefix, sourceCategories, validateFinalReview,
  validateNormalizedFinalReview,
};
