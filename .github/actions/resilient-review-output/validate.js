"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const { MAX_FILES } = require("../../pr-automation/deterministic-analysis");
const { invalid, REPO_PATH, SHA } = require("../../pr-automation/validation");
const {
  corpusFromDirectory, validateProtocolReview,
} = require("../../pr-automation/validate-protocol-review");
const { validateReviewer } = require("../../pr-automation/validate-reviewer");

const MAX_PATH_BYTES = 500;

function parseChangedPaths(source) {
  if (!Buffer.isBuffer(source) || source.length === 0 ||
      source.length > MAX_FILES * (MAX_PATH_BYTES + 1) || source.at(-1) !== 0) {
    return invalid("invalid changed path evidence");
  }
  const paths = source.toString("utf8").slice(0, -1).split("\0");
  if (paths.length === 0 || paths.length > MAX_FILES || new Set(paths).size !== paths.length ||
      paths.some((entry) => !REPO_PATH.test(entry) || Buffer.byteLength(entry, "utf8") > MAX_PATH_BYTES ||
        /[\u0000-\u001F\u007F]/.test(entry))) {
    return invalid("invalid changed path evidence");
  }
  return { ok: true, paths };
}

function changedPathsFromRepository(repository) {
  try {
    return parseChangedPaths(execFileSync("git", [
      "-C", repository, "diff", "--find-renames", "--name-only", "-z", "origin/master...HEAD",
    ], { encoding: "buffer", maxBuffer: MAX_FILES * (MAX_PATH_BYTES + 1) }));
  } catch {
    return invalid("changed path evidence unavailable");
  }
}

function validateModelOutput(raw, {
  stage, expectedSha, workspace = process.cwd(), changedPaths, corpus, protocolReceived,
} = {}) {
  if (!["protocol-analysis", "skeptical-review"].includes(stage) || !SHA.test(expectedSha || "")) {
    return invalid("invalid model validation context");
  }
  const paths = changedPaths === undefined
    ? changedPathsFromRepository(path.join(workspace, "pr-head"))
    : { ok: true, paths: changedPaths };
  if (!paths.ok) return paths;
  if (stage === "protocol-analysis") {
    return validateProtocolReview(raw, {
      expectedSha,
      changedPaths: paths.paths,
      corpus: corpus ?? corpusFromDirectory(path.join(workspace, ".claude", "skills", "windows-protocols")),
    });
  }
  if (protocolReceived === undefined) {
    try {
      protocolReceived = JSON.parse(fs.readFileSync(path.join(workspace, "protocol-handoff.json"), "utf8")) !== null;
    } catch {
      return invalid("protocol handoff evidence unavailable");
    }
  }
  return validateReviewer(raw, {
    expectedSha, changedPaths: paths.paths, changedLines: {}, protocolReceived,
  });
}

module.exports = { changedPathsFromRepository, parseChangedPaths, validateModelOutput };
