"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const { MAX_FILES } = require("../../pr-automation/deterministic-analysis");
const { invalid, isPlainObject, REPO_PATH, SHA } = require("../../pr-automation/validation");
const {
  corpusFromDirectory, validateProtocolReview,
} = require("../../pr-automation/validate-protocol-review");
const { validateClassifier } = require("../../pr-automation/validate-classifier");
const { validateReviewer } = require("../../pr-automation/validate-reviewer");

const MAX_PATH_BYTES = 500;
const MAX_EXECUTION_BYTES = 64 * 1024 * 1024;
const SESSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function isSessionId(value) {
  return typeof value === "string" && SESSION_ID.test(value);
}

function recoverExecutionOutput(runnerTemp) {
  if (typeof runnerTemp !== "string" || runnerTemp.length === 0) {
    return invalid("execution transcript path unavailable");
  }
  const executionFile = path.join(runnerTemp, "claude-execution-output.json");
  let messages;
  try {
    const size = fs.statSync(executionFile).size;
    if (size === 0 || size > MAX_EXECUTION_BYTES) {
      return invalid("execution transcript size is invalid");
    }
    messages = JSON.parse(fs.readFileSync(executionFile, "utf8"));
  } catch {
    return invalid("execution transcript unavailable");
  }
  if (!Array.isArray(messages)) return invalid("execution transcript is invalid");

  let sessionId = "";
  let structuredOutput = "";
  for (const message of messages) {
    if (!isPlainObject(message)) continue;
    if (message.type === "system" && message.subtype === "init" &&
        isSessionId(message.session_id)) {
      sessionId = message.session_id;
    }
    if (message.type === "result" && message.subtype === "success" && message.is_error === false &&
        Object.hasOwn(message, "structured_output") && message.structured_output !== null) {
      structuredOutput = typeof message.structured_output === "string"
        ? message.structured_output
        : JSON.stringify(message.structured_output);
    }
  }
  return { ok: true, sessionId, structuredOutput };
}

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
  stage, expectedSha, workspace = process.cwd(), changedPaths, corpus, protocolReceived, prNumber,
} = {}) {
  if (!["classifier", "protocol-analysis", "skeptical-review"].includes(stage) || !SHA.test(expectedSha || "")) {
    return invalid("invalid model validation context");
  }
  if (stage === "classifier" && (!Number.isSafeInteger(prNumber) || prNumber < 1)) {
    return invalid("invalid classifier validation context");
  }
  const paths = changedPaths === undefined
    ? changedPathsFromRepository(path.join(workspace, "pr-head"))
    : { ok: true, paths: changedPaths };
  if (!paths.ok) return paths;
  if (stage === "classifier") {
    return validateClassifier(raw, {
      expectedSha, changedPaths: paths.paths, prNumber,
    });
  }
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

module.exports = {
  changedPathsFromRepository, isSessionId, parseChangedPaths, recoverExecutionOutput, validateModelOutput,
};
