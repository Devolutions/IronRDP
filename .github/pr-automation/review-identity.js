"use strict";

// Content-addressed identity for reviewer stages.
//
// A stage result may only be reused when every input that could change it is identical: the reviewed
// commits, the exact evidence bytes, the trusted rules that produced the result, the protocol corpus
// commit, and, for the general stage, the specialist aggregate it depends on.
//
// The policy digest deliberately covers a declared file manifest rather than `github.workflow_sha`.
// Binding to the workflow commit would invalidate every cached result whenever an unrelated commit
// lands on the base branch, which would defeat recovery entirely.

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const IDENTITY_VERSION = 1;
const MAXIMUM_TREE_FILES = 100000;
const SHA = /^[0-9a-f]{40}$/;
const DIGEST = /^[0-9a-f]{64}$/;

// Trusted files that decide how a reviewer behaves. Any change to one must invalidate reuse.
const POLICY_FILES = Object.freeze([
  ".github/actions/openai-agent/dist/index.js",
  ".github/pr-automation/agent-validator.js",
  ".github/pr-automation/review-identity.js",
  ".github/pr-automation/review-pipeline.js",
  ".github/pr-automation/review-reuse.js",
  ".github/pr-automation/routing.js",
  ".github/pr-automation/validate-candidate-review.js",
  ".github/pr-automation/validate-final-review.js",
  ".github/pr-automation/validate-protocol-review.js",
  ".github/pr-automation/validation.js",
  ".github/workflows/review-pipeline.yml",
]);

const POLICY_DIRECTORIES = Object.freeze([
  ".github/pr-automation/agents",
  ".github/pr-automation/prompts",
  ".github/pr-automation/schemas",
]);

const AGENT_DIRECTORY = ".github/pr-automation/agents";

const EVIDENCE_FILES = Object.freeze([
  "pr-evidence/changed-files.txt",
  "pr-evidence/pull-request.diff",
  "pr-evidence/pull-request-context.json",
  "pr-evidence/validation-context.json",
]);

const EVIDENCE_DIRECTORIES = Object.freeze(["pr-head"]);

class IdentityError extends Error {}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function safeRelative(relative) {
  if (typeof relative !== "string" || relative.length === 0 || relative.length > 300 ||
      relative.startsWith("/") || relative.includes("\\") || /(?:^|\/)\.\.(?:\/|$)/.test(relative)) {
    throw new IdentityError(`unsafe identity path: ${String(relative).slice(0, 60)}`);
  }
  return relative;
}

function hashFile(root, relative) {
  const target = path.resolve(root, safeRelative(relative));
  let metadata;
  try {
    metadata = fs.lstatSync(target);
  } catch {
    throw new IdentityError(`identity input is missing: ${relative}`);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    throw new IdentityError(`identity input is not a regular file: ${relative}`);
  }
  return sha256(fs.readFileSync(target));
}

function hashTree(root, relativeDirectory, entries) {
  const base = safeRelative(relativeDirectory);
  const walk = (relative) => {
    const absolute = path.resolve(root, relative);
    for (const entry of fs.readdirSync(absolute, { withFileTypes: true }).sort(
      (left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0,
    )) {
      const child = `${relative}/${entry.name}`;
      if (entry.isSymbolicLink()) throw new IdentityError(`identity tree contains a symlink: ${child}`);
      if (entry.isDirectory()) {
        walk(child);
        continue;
      }
      if (!entry.isFile()) throw new IdentityError(`identity tree contains a special file: ${child}`);
      if (entries.length >= MAXIMUM_TREE_FILES) throw new IdentityError("identity tree is too large");
      entries.push([child, sha256(fs.readFileSync(path.resolve(root, child)))]);
    }
  };
  try {
    if (!fs.lstatSync(path.resolve(root, base)).isDirectory()) {
      throw new IdentityError(`identity input is not a directory: ${base}`);
    }
  } catch (error) {
    if (error instanceof IdentityError) throw error;
    throw new IdentityError(`identity input is missing: ${base}`);
  }
  walk(base);
}

function digestOfEntries(entries) {
  const sorted = [...entries].sort(([left], [right]) =>
    left < right ? -1 : left > right ? 1 : 0);
  return sha256(sorted.map(([relative, hash]) => `${relative}\0${hash}\n`).join(""));
}

// Methodology files live outside the policy directories but decide reviewer behaviour just as much.
function methodologyFiles(root) {
  const directory = path.resolve(root, AGENT_DIRECTORY);
  const files = new Set();
  for (const name of fs.readdirSync(directory).sort()) {
    if (!name.endsWith(".json")) continue;
    let config;
    try {
      config = JSON.parse(fs.readFileSync(path.join(directory, name), "utf8"));
    } catch {
      throw new IdentityError(`agent configuration is not valid JSON: ${name}`);
    }
    for (const file of config?.methodology_files ?? []) files.add(safeRelative(file));
  }
  return [...files].sort();
}

function policyDigest(root) {
  const entries = [];
  for (const file of POLICY_FILES) entries.push([file, hashFile(root, file)]);
  for (const directory of POLICY_DIRECTORIES) hashTree(root, directory, entries);
  for (const file of methodologyFiles(root)) entries.push([file, hashFile(root, file)]);
  return digestOfEntries(entries);
}

function evidenceDigest(root) {
  const entries = [];
  for (const file of EVIDENCE_FILES) entries.push([file, hashFile(root, file)]);
  for (const directory of EVIDENCE_DIRECTORIES) hashTree(root, directory, entries);
  return digestOfEntries(entries);
}

function fileDigest(root, relative) {
  return hashFile(root, relative);
}

function requireSha(value, label) {
  if (!SHA.test(value || "")) throw new IdentityError(`invalid ${label}`);
  return value;
}

function requireDigest(value, label) {
  if (!DIGEST.test(value || "")) throw new IdentityError(`invalid ${label}`);
  return value;
}

// The canonical key every reuse decision is made against.
function stageKey({
  stage, baseSha, headSha, evidenceDigest: evidence, policyDigest: policy,
  corpusSha = null, aggregateDigest = null,
} = {}) {
  if (typeof stage !== "string" || !/^(?:specialist:[a-z][a-z0-9-]{0,63}|general)$/.test(stage)) {
    throw new IdentityError("invalid identity stage");
  }
  return sha256(JSON.stringify({
    v: IDENTITY_VERSION,
    stage,
    base_sha: requireSha(baseSha, "identity base SHA"),
    head_sha: requireSha(headSha, "identity head SHA"),
    evidence_digest: requireDigest(evidence, "identity evidence digest"),
    policy_digest: requireDigest(policy, "identity policy digest"),
    corpus_sha: corpusSha === null ? null : requireSha(corpusSha, "identity corpus SHA"),
    aggregate_digest: aggregateDigest === null
      ? null
      : requireDigest(aggregateDigest, "identity aggregate digest"),
  }));
}

module.exports = {
  DIGEST, EVIDENCE_DIRECTORIES, EVIDENCE_FILES, IDENTITY_VERSION, IdentityError,
  POLICY_DIRECTORIES, POLICY_FILES,
  evidenceDigest, fileDigest, methodologyFiles, policyDigest, stageKey,
};
