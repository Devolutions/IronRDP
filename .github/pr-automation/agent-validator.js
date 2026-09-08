"use strict";

// Review-specific validator handed to the model runtime.
//
// The runtime knows nothing about pull requests: it validates JSON and the output schema, then calls
// this trusted module. Rejections here become bounded repair feedback inside the same conversation,
// so the model can correct a wrong SHA, an unchanged path, an unvalidated line range, a bad protocol
// citation, or a missing candidate disposition without replaying its investigation.
//
// Two outcomes are deliberately different:
//
// - `{ ok: false, reason }` is repairable model output.
// - A thrown `VALIDATOR_TERMINAL` error means a trusted input is stale or unavailable. Repair cannot
//   fix that, so the stage fails instead of burning repair attempts.

const fs = require("node:fs");

const { REVIEWER_ORDER } = require("./routing");
const { normalizeText } = require("./validation");
const { corpusFromDirectory, validateProtocolReferences } = require("./validate-protocol-review");
const { normalizeCandidateReview } = require("./validate-candidate-review");
const { validateFinalReview } = require("./validate-final-review");

const TERMINAL_CODE = "VALIDATOR_TERMINAL";
const SHA = /^[0-9a-f]{40}$/;
const MAXIMUM_TRUSTED_BYTES = 8 * 1024 * 1024;

// Read from the schemas the model answers to, so a repair baseline is judged by the same rules.
const CANDIDATE_LIMITS = require("./schemas/candidate-review.json").properties.findings;
const FINAL_LIMITS = require("./schemas/final-review.json").properties.findings;
const CANDIDATE_FINDING_ID = new RegExp(CANDIDATE_LIMITS.items.properties.id.pattern);
// The review validators cap a title at 200 UTF-8 bytes, which is stricter than the schema's
// character limit, so the byte limit is what a repair can actually reach.
const MAX_TITLE_BYTES = 200;
const DISPOSITION_REVIEWERS = require("./schemas/final-review.json")
  .properties.candidate_dispositions.items.properties.reviewer.enum;

function terminal(reason) {
  const error = new Error(reason);
  error.code = TERMINAL_CODE;
  return error;
}

// The runtime accepts a bounded rejection alphabet and turns anything else into a terminal failure,
// so model-controlled text is scrubbed here instead of costing the stage its repair attempts.
function reject(reason) {
  const text = String(reason ?? "")
    .replace(/[^A-Za-z0-9 .,:;()\/_-]/g, " ")
    .replace(/ +/g, " ")
    .trim();
  const start = text.search(/[A-Za-z0-9]/);
  return {
    ok: false,
    reason: start === -1 ? "the review output was rejected" : text.slice(start, start + 512),
  };
}

function requireString(metadata, key, pattern) {
  const value = metadata?.[key];
  if (typeof value !== "string" || (pattern && !pattern.test(value))) {
    throw terminal(`validator metadata is missing ${key}`);
  }
  return value;
}

// The pipeline produces these files earlier in the same run, so they are authoritative by origin
// and need no separate integrity check. What they still have to survive is being missing,
// truncated, or replaced by something that is not a readable regular file.
function readTrustedJson(file, label) {
  let raw;
  try {
    const metadata = fs.lstatSync(file);
    if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > MAXIMUM_TRUSTED_BYTES) {
      throw new Error("not a trusted regular file");
    }
    raw = fs.readFileSync(file);
  } catch {
    throw terminal(`${label} is unavailable`);
  }
  try {
    return JSON.parse(raw.toString("utf8"));
  } catch {
    throw terminal(`${label} is not valid JSON`);
  }
}

function loadValidationContext(metadata) {
  const file = requireString(metadata, "validation_context_file");
  const context = readTrustedJson(file, "the changed-file manifest");
  if (!Array.isArray(context?.changed_paths) || context.changed_paths.length === 0 ||
      context.changed_lines === null || typeof context.changed_lines !== "object") {
    throw terminal("the changed-file manifest is unusable");
  }
  return context;
}

function loadCorpus(metadata) {
  const directory = requireString(metadata, "corpus_dir");
  const corpusSha = requireString(metadata, "corpus_sha", SHA);
  const corpus = corpusFromDirectory(directory, { expectedCorpusSha: corpusSha });
  if (!corpus.isPinnedTo(corpusSha)) {
    throw terminal("the protocol corpus is unavailable or not pinned to the expected commit");
  }
  return { corpus, corpusSha };
}

// The coarse validators are the authority on acceptance. They report a category, which is correct for
// a failure record but too vague to repair from, so name the offending field when we can.
function diagnoseCandidate(candidate, { expectedSha, reviewer, changedPaths }) {
  if (candidate?.head_sha !== expectedSha) {
    return `head_sha must be exactly ${expectedSha}`;
  }
  if (candidate?.reviewer !== reviewer) {
    return `reviewer must be exactly ${reviewer}`;
  }
  const findings = Array.isArray(candidate?.findings) ? candidate.findings : [];
  for (const finding of findings) {
    const id = typeof finding?.id === "string" ? finding.id : "an unnamed finding";
    if (typeof finding?.path !== "string" || !changedPaths.has(finding.path)) {
      return `finding ${id} must cite a path changed by this pull request`;
    }
    const linesAreNull = finding.start_line === null && finding.end_line === null;
    const linesAreIntegers = Number.isSafeInteger(finding.start_line) && finding.start_line >= 1 &&
      Number.isSafeInteger(finding.end_line) && finding.end_line >= finding.start_line;
    if (!linesAreNull && !linesAreIntegers) {
      return `finding ${id} must use integer lines with end_line at or after start_line, or null lines`;
    }
    if (reviewer !== "protocol" && Array.isArray(finding.references) && finding.references.length > 0) {
      return `finding ${id} must not carry protocol references`;
    }
  }
  return "";
}

function diagnoseProtocolReferences(candidate, corpus, corpusSha) {
  for (const finding of candidate?.findings ?? []) {
    const id = typeof finding?.id === "string" ? finding.id : "an unnamed finding";
    if (!Array.isArray(finding?.references) || finding.references.length === 0) {
      return `finding ${id} must cite at least one section of the pinned protocol corpus`;
    }
    const result = validateProtocolReferences(finding.references, {
      corpus, expectedCorpusSha: corpusSha,
    });
    if (!result.ok) {
      return `finding ${id} cites a protocol section that does not exist in the pinned corpus`;
    }
  }
  return "";
}

// The runtime keeps the model's first response as the repair baseline even when that response failed
// the output schema, so a baseline entry the schema itself rejects is never demanded back: restoring
// it could not pass either. Everything the schema would have accepted still has to survive.
function findingCounts(review, identify, distinct) {
  const counts = new Map();
  for (const finding of Array.isArray(review?.findings) ? review.findings : []) {
    const key = identify(finding);
    if (key !== null) counts.set(key, distinct ? 1 : (counts.get(key) ?? 0) + 1);
  }
  return counts;
}

function droppedFindings(current, previous, { identify, maximum, distinct = false }) {
  const findings = Array.isArray(previous?.findings) ? previous.findings : [];
  if (findings.length > maximum) return [];
  const kept = findingCounts(current, identify, distinct);
  const dropped = [];
  for (const [key, count] of findingCounts(previous, identify, distinct)) {
    if ((kept.get(key) ?? 0) < count) dropped.push(key);
  }
  return dropped;
}

const candidateIdentity = (finding) =>
  typeof finding?.id === "string" && CANDIDATE_FINDING_ID.test(finding.id) ? finding.id : null;

// Final findings carry no id, so a title identifies them. Repair corrects a citation, a path, or a
// line range, never the issue a finding reports. The title is normalized exactly as the review
// validators do, so a title only they would reject is never protected.
const finalIdentity = (finding) => {
  const title = normalizeText(finding?.title, MAX_TITLE_BYTES);
  return title ? title.toLowerCase() : null;
};

function preservedCandidateFindings(candidate, previousCandidate) {
  if (!previousCandidate) return "";
  const dropped = droppedFindings(candidate, previousCandidate, {
    identify: candidateIdentity, maximum: CANDIDATE_LIMITS.maxItems,
    // The candidate validator rejects a repeated id, so a repair can only ever keep one of them.
    distinct: true,
  });
  return dropped.length === 0
    ? ""
    : `repair must keep every earlier finding; restore ${dropped.slice(0, 5).join(", ")} and correct it instead of removing it`;
}

const dispositionKey = (reviewer, findingId) => `${reviewer}\u0000${findingId}`;

// A disposition is only preserved when it names a candidate the validated specialists actually
// produced. The final validator rejects any other one, so repair has to drop it.
function aggregateCandidateKeys(aggregate) {
  const keys = new Set();
  for (const review of Array.isArray(aggregate?.reviewers) ? aggregate.reviewers : []) {
    if (review?.status !== "valid") continue;
    for (const finding of Array.isArray(review.findings) ? review.findings : []) {
      const id = candidateIdentity(finding);
      if (id !== null) keys.add(dispositionKey(review.reviewer, id));
    }
  }
  return keys;
}

function acceptedKeys(review, candidates) {
  const dispositions = review?.candidate_dispositions;
  return new Set((Array.isArray(dispositions) ? dispositions : [])
    .filter((entry) => entry?.disposition === "accepted" || entry?.disposition === "refined")
    .filter((entry) => DISPOSITION_REVIEWERS.includes(entry?.reviewer) &&
      typeof entry?.finding_id === "string" && CANDIDATE_FINDING_ID.test(entry.finding_id))
    .map((entry) => dispositionKey(entry.reviewer, entry.finding_id))
    .filter((key) => candidates.has(key)));
}

function preservedFinalFindings(review, previousReview, candidates) {
  if (!previousReview) return "";
  const current = acceptedKeys(review, candidates);
  const withdrawn = [...acceptedKeys(previousReview, candidates)].filter((key) => !current.has(key));
  if (withdrawn.length > 0) {
    return "repair must not reject a candidate it previously accepted or refined; correct the finding instead";
  }
  const dropped = droppedFindings(review, previousReview, {
    identify: finalIdentity, maximum: FINAL_LIMITS.maxItems,
  });
  return dropped.length === 0
    ? ""
    : "repair must keep every earlier finding; restore the one it dropped and correct it instead of replacing it";
}

function validateSpecialist(candidate, { metadata, previousCandidate } = {}) {
  if (metadata?.stage !== "specialist") throw terminal("validator metadata is not a specialist stage");
  const reviewer = requireString(metadata, "reviewer");
  if (!REVIEWER_ORDER.includes(reviewer)) throw terminal("validator metadata names an unknown reviewer");
  const expectedSha = requireString(metadata, "expected_sha", SHA);
  const context = loadValidationContext(metadata);
  const changedPaths = new Set(context.changed_paths);

  const preserved = preservedCandidateFindings(candidate, previousCandidate);
  if (preserved) return reject(preserved);

  const result = normalizeCandidateReview(candidate, {
    expectedSha,
    expectedReviewer: reviewer,
    changedPaths: context.changed_paths,
    changedLines: context.changed_lines,
  });
  if (!result.ok) {
    return reject(diagnoseCandidate(candidate, {
      expectedSha, reviewer, changedPaths,
    }) || result.reason);
  }

  if (reviewer === "protocol") {
    const { corpus, corpusSha } = loadCorpus(metadata);
    const diagnosis = diagnoseProtocolReferences(result.value, corpus, corpusSha);
    if (diagnosis) return reject(diagnosis);
  }
  return { ok: true };
}

function validateGeneral(review, { metadata, previousCandidate } = {}) {
  if (metadata?.stage !== "general") throw terminal("validator metadata is not a general stage");
  const expectedSha = requireString(metadata, "expected_sha", SHA);
  const context = loadValidationContext(metadata);
  const aggregate = readTrustedJson(
    requireString(metadata, "aggregate_file"),
    "the validated specialist findings",
  );

  const preserved = preservedFinalFindings(review, previousCandidate, aggregateCandidateKeys(aggregate));
  if (preserved) return reject(preserved);

  const result = validateFinalReview(review, {
    expectedSha,
    changedPaths: context.changed_paths,
    changedLines: context.changed_lines,
    specialistAggregate: aggregate,
  });
  if (result.ok) return { ok: true };

  if (result.reason === "validated specialist findings unavailable") {
    throw terminal("the validated specialist findings are stale or unusable");
  }
  if (review?.head_sha !== expectedSha) {
    return reject(`head_sha must be exactly ${expectedSha}`);
  }
  const changedPaths = new Set(context.changed_paths);
  for (const finding of review?.findings ?? []) {
    const title = typeof finding?.title === "string" ? finding.title.slice(0, 60) : "an untitled finding";
    if (typeof finding?.path !== "string" || !changedPaths.has(finding.path)) {
      return reject(`finding ${title} must cite a path changed by this pull request`);
    }
  }
  return reject(`${result.reason}; record exactly one disposition per specialist candidate and cite only non-rejected candidates as sources`);
}

module.exports = {
  TERMINAL_CODE, validateGeneral, validateSpecialist,
};
