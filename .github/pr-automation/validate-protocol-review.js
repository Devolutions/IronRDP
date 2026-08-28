"use strict";

const fs = require("node:fs");
const path = require("node:path");

const { SHA, exactKeys, invalid, isBoundedArray, normalizeText } = require("./validation");

const PROTOCOL_ID = /^(?:MS|MC)-[A-Z0-9]+$/;
const SECTION = /^[0-9]+(?:\.[0-9]+)*$/;
const CORPUS_COMMIT_FILE = ".corpus-commit";

function corpusFromDirectory(directory, { expectedCorpusSha } = {}) {
  const commit = readCorpusCommit(directory);
  const pinned = expectedCorpusSha === undefined ||
    (SHA.test(expectedCorpusSha) && commit === expectedCorpusSha);
  const cache = new Map();
  const headings = (protocolId) => {
    if (!pinned || !PROTOCOL_ID.test(protocolId)) return null;
    if (!cache.has(protocolId)) cache.set(protocolId, indexSections(directory, protocolId));
    return cache.get(protocolId);
  };
  return {
    isPinnedTo: (candidate) => SHA.test(candidate) && commit === candidate,
    headingOf: (protocolId, section) => headings(protocolId)?.get(section) ?? null,
  };
}

function readCorpusCommit(directory) {
  try {
    const root = fs.lstatSync(directory);
    const commitFile = path.join(directory, CORPUS_COMMIT_FILE);
    const metadata = fs.lstatSync(commitFile);
    if (!root.isDirectory() || root.isSymbolicLink() ||
        !metadata.isFile() || metadata.isSymbolicLink()) return null;
    const commit = fs.readFileSync(commitFile, "utf8").trim();
    return SHA.test(commit) ? commit : null;
  } catch {
    return null;
  }
}

function indexSections(directory, protocolId) {
  let lines;
  try {
    const root = fs.realpathSync(directory);
    const protocolDirectory = path.join(root, protocolId);
    const corpusFile = path.join(protocolDirectory, `${protocolId}.md`);
    const protocolMetadata = fs.lstatSync(protocolDirectory);
    const fileMetadata = fs.lstatSync(corpusFile);
    if (!protocolMetadata.isDirectory() || protocolMetadata.isSymbolicLink() ||
        !fileMetadata.isFile() || fileMetadata.isSymbolicLink() ||
        !fs.realpathSync(corpusFile).startsWith(`${root}${path.sep}`)) return null;
    lines = fs.readFileSync(corpusFile, "utf8").split(/\r?\n/);
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

function validateProtocolReferences(entries, { corpus, expectedCorpusSha } = {}) {
  if (!corpus?.isPinnedTo?.(expectedCorpusSha)) {
    return invalid("protocol corpus commit mismatch");
  }
  if (!isBoundedArray(entries, 5) || entries.length === 0) {
    return invalid("invalid protocol reference array");
  }
  const references = [];
  const seen = new Set();
  for (const entry of entries) {
    if (!exactKeys(entry, ["protocol_id", "section", "heading"]) ||
        !PROTOCOL_ID.test(entry.protocol_id) || !SECTION.test(entry.section)) {
      return invalid("invalid protocol reference");
    }
    const heading = normalizeText(entry.heading, 200);
    const expectedHeading = corpus.headingOf(entry.protocol_id, entry.section);
    const key = `${entry.protocol_id}\0${entry.section}`;
    if (!heading || expectedHeading === null || seen.has(key) ||
        comparableHeading(expectedHeading) !== comparableHeading(heading)) {
      return invalid("invalid protocol reference");
    }
    seen.add(key);
    references.push({
      protocol_id: entry.protocol_id,
      section: entry.section,
      heading,
    });
  }
  return { ok: true, status: "valid", value: references };
}

module.exports = {
  CORPUS_COMMIT_FILE, PROTOCOL_ID, SECTION,
  corpusFromDirectory, validateProtocolReferences,
};
