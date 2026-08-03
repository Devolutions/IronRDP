"use strict";

const fs = require("node:fs");
const path = require("node:path");

const { REPO_PATH, SHA, exactKeys, invalid, isBoundedArray, normalizeText, parseJson } = require("./validation");

const SCHEMA_VERSION = "protocol-reviewer-v1";
const PROTOCOL_ID = /^(?:MS|MC)-[A-Z0-9]+$/;
const SECTION = /^[0-9]+(?:\.[0-9]+)*$/;
const MAXIMUM_BYTES = 49152;

// Reads the pinned Open Specifications corpus so cited protocol IDs and section numbers can be
// checked against real headings. This proves the citation exists; it never proves the model read it
// correctly, so cited text remains untrusted evidence.
function corpusFromDirectory(directory) {
  const cache = new Map();
  const headings = (protocolId) => {
    if (!PROTOCOL_ID.test(protocolId)) return null;
    if (!cache.has(protocolId)) cache.set(protocolId, indexSections(directory, protocolId));
    return cache.get(protocolId);
  };
  return {
    hasProtocol: (protocolId) => headings(protocolId) !== null,
    headingOf: (protocolId, section) => headings(protocolId)?.get(section) ?? null,
  };
}

// Sections nested more than six levels deep are not Markdown headings in this corpus: they are an
// anchor followed by a bare title line. Those are exactly the leaf wire-format structures IronRDP
// changes, so both shapes must be indexed.
function indexSections(directory, protocolId) {
  let lines;
  try {
    lines = fs.readFileSync(path.join(directory, protocolId, `${protocolId}.md`), "utf8").split(/\r?\n/);
  } catch {
    return null;
  }
  const sections = new Map();
  const anchors = new Map();
  lines.forEach((line, index) => {
    const heading = /^#{1,6}\s+([0-9]+(?:\.[0-9]+)*)\s+(.+?)\s*$/.exec(line);
    if (heading) {
      if (!sections.has(heading[1])) sections.set(heading[1], heading[2]);
      return;
    }
    const anchor = /^<a id="Section_([0-9]+(?:\.[0-9]+)*)"><\/a>$/.exec(line.trim());
    if (!anchor || anchors.has(anchor[1])) return;
    const title = lines.slice(index + 1, index + 4).map((entry) => entry.trim()).find(Boolean);
    if (title && !title.startsWith("#")) anchors.set(anchor[1], title);
  });
  for (const [section, title] of anchors) if (!sections.has(section)) sections.set(section, title);
  return sections;
}

function comparableHeading(value) {
  return String(value).toLowerCase().replace(/\s+/g, " ").replace(/[.:]+$/, "").trim();
}

function citationExists(corpus, protocolId, section) {
  return PROTOCOL_ID.test(protocolId) && SECTION.test(section) &&
    corpus.hasProtocol(protocolId) && corpus.headingOf(protocolId, section) !== null;
}

function normalizeConsulted(entries, corpus) {
  const normalized = [];
  for (const entry of entries) {
    if (!exactKeys(entry, ["protocol_id", "section", "heading"])) return null;
    const heading = normalizeText(entry.heading, 200);
    if (!heading || !citationExists(corpus, entry.protocol_id, entry.section) ||
        comparableHeading(corpus.headingOf(entry.protocol_id, entry.section)) !== comparableHeading(heading)) {
      return null;
    }
    normalized.push({ protocol_id: entry.protocol_id, section: entry.section, heading });
  }
  return normalized;
}

function normalizeMappings(entries, corpus, changedPaths) {
  const keys = ["path", "line", "symbol", "change", "requirement", "source_protocol",
    "source_section", "assessment", "confidence", "evidence"];
  const normalized = [];
  for (const entry of entries) {
    if (!exactKeys(entry, keys) ||
        typeof entry.path !== "string" || !REPO_PATH.test(entry.path) || !changedPaths.has(entry.path) ||
        !(entry.line === null || (Number.isSafeInteger(entry.line) && entry.line >= 1)) ||
        !["conforms", "conflicts", "incomplete_evidence", "ambiguous"].includes(entry.assessment) ||
        !["high", "medium", "low"].includes(entry.confidence) ||
        !citationExists(corpus, entry.source_protocol, entry.source_section)) {
      return null;
    }
    const symbol = entry.symbol === null ? null : normalizeText(entry.symbol, 200);
    const change = normalizeText(entry.change, 600);
    const requirement = normalizeText(entry.requirement, 600);
    const evidence = normalizeText(entry.evidence, 1200);
    if (symbol === null && entry.symbol !== null) return null;
    if (!change || !requirement || !evidence) return null;
    normalized.push({
      path: entry.path, line: entry.line, symbol, change, requirement,
      source_protocol: entry.source_protocol, source_section: entry.source_section,
      assessment: entry.assessment, confidence: entry.confidence, evidence,
    });
  }
  return normalized;
}

function normalizeDiscrepancies(entries, corpus) {
  const normalized = [];
  for (const entry of entries) {
    if (!exactKeys(entry, ["description", "affected_behavior", "protocol_id", "section"]) ||
        !citationExists(corpus, entry.protocol_id, entry.section)) return null;
    const description = normalizeText(entry.description, 800);
    const affected = normalizeText(entry.affected_behavior, 400);
    if (!description || !affected) return null;
    normalized.push({
      description, affected_behavior: affected, protocol_id: entry.protocol_id, section: entry.section,
    });
  }
  return normalized;
}

function normalizeNotes(entries) {
  const normalized = [];
  for (const entry of entries) {
    const note = normalizeText(entry, 300);
    if (!note) return null;
    normalized.push(note);
  }
  return normalized;
}

// Trusted result used when the classifier reported no protocol relevance: no model was invoked.
function notApplicableHandoff() {
  return { ok: true, status: "not_applicable", schemaVersion: SCHEMA_VERSION, value: null };
}

function validateProtocolReview(raw, { expectedSha, changedPaths = [], corpus } = {}) {
  if (!corpus) return invalid("protocol corpus unavailable");
  const value = parseJson(raw, MAXIMUM_BYTES);
  const keys = ["schema_version", "head_sha", "protocol_relevance", "relevance_reason",
    "protocols_consulted", "change_mappings", "potential_discrepancies",
    "required_or_valuable_tests", "uncertainty"];
  if (!exactKeys(value, keys) || value.schema_version !== "1" ||
      !SHA.test(value.head_sha) || value.head_sha !== expectedSha ||
      !["none", "low", "medium", "high"].includes(value.protocol_relevance) ||
      !isBoundedArray(value.protocols_consulted, 20) || !isBoundedArray(value.change_mappings, 30) ||
      !isBoundedArray(value.potential_discrepancies, 20) ||
      !isBoundedArray(value.required_or_valuable_tests, 20) || !isBoundedArray(value.uncertainty, 20)) {
    return invalid("invalid protocol review object");
  }
  const reason = normalizeText(value.relevance_reason, 500);
  if (!reason) return invalid("invalid protocol relevance reason");

  const isNone = value.protocol_relevance === "none";
  const populated = value.protocols_consulted.length + value.change_mappings.length +
    value.potential_discrepancies.length + value.required_or_valuable_tests.length;
  if (isNone ? populated !== 0 : value.protocols_consulted.length === 0) {
    return invalid("protocol relevance contradicts reported evidence");
  }

  const protocolsConsulted = normalizeConsulted(value.protocols_consulted, corpus);
  const changeMappings = protocolsConsulted &&
    normalizeMappings(value.change_mappings, corpus, new Set(changedPaths));
  const potentialDiscrepancies = changeMappings && normalizeDiscrepancies(value.potential_discrepancies, corpus);
  const tests = potentialDiscrepancies && normalizeNotes(value.required_or_valuable_tests);
  const uncertainty = tests && normalizeNotes(value.uncertainty);
  if (!uncertainty) return invalid("invalid protocol citation, location, or text");

  const normalized = {
    schema_version: value.schema_version, head_sha: value.head_sha,
    protocol_relevance: value.protocol_relevance, relevance_reason: reason,
    protocols_consulted: protocolsConsulted, change_mappings: changeMappings,
    potential_discrepancies: potentialDiscrepancies,
    required_or_valuable_tests: tests, uncertainty,
  };
  if (Buffer.byteLength(JSON.stringify(normalized), "utf8") > MAXIMUM_BYTES) {
    return invalid("protocol review output too large");
  }
  return { ok: true, status: "valid", schemaVersion: SCHEMA_VERSION, value: normalized };
}

module.exports = {
  PROTOCOL_ID, SCHEMA_VERSION,
  corpusFromDirectory, notApplicableHandoff, validateProtocolReview,
};
