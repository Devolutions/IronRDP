"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const { ValidatorFailure, loadValidator, parseMetadata } = require("../src/validator");
const { scratchWorkspace, write } = require("./helpers");

test("trusted validator receives opaque metadata and prior candidates", async () => {
  const workspace = scratchWorkspace();
  write(workspace.directory, "validator.js", [
    "exports.validate = (candidate, context) => ({",
    "  ok: candidate.answer === context.metadata.expected && context.previousCandidate?.answer === 'old',",
    "  reason: 'candidate does not preserve the expected answer',",
    "});",
  ].join("\n"));
  try {
    const metadata = parseMetadata('{"expected":"new"}');
    const validate = loadValidator(workspace.directory, "validator.js#validate", metadata);
    assert.deepEqual(
      await validate({ answer: "new" }, { previousCandidate: { answer: "old" }, repairAttempt: 1 }),
      { ok: true },
    );
    assert.deepEqual(
      await validate({ answer: "wrong" }, { previousCandidate: { answer: "old" }, repairAttempt: 1 }),
      { ok: false, reason: "candidate does not preserve the expected answer" },
    );
  } finally {
    workspace.cleanup();
  }
});

test("validator metadata and execution failures are bounded and classified", async () => {
  assert.throws(() => parseMetadata("[]"));
  assert.throws(() => parseMetadata("{"));

  const workspace = scratchWorkspace();
  write(workspace.directory, "validator.js", [
    "exports.terminal = () => {",
    "  const error = new Error('trusted input is unavailable');",
    "  error.code = 'VALIDATOR_TERMINAL';",
    "  throw error;",
    "};",
    "exports.crash = () => { throw new Error('MODEL_SECRET_SENTINEL'); };",
  ].join("\n"));
  try {
    const terminal = loadValidator(workspace.directory, "validator.js#terminal", {});
    await assert.rejects(
      terminal({}, { previousCandidate: null, repairAttempt: 0 }),
      (error) => error instanceof ValidatorFailure &&
        error.category === "validator-terminal" &&
        error.reason === "trusted input is unavailable",
    );
    const crash = loadValidator(workspace.directory, "validator.js#crash", {});
    await assert.rejects(
      crash({}, { previousCandidate: null, repairAttempt: 0 }),
      (error) => error instanceof ValidatorFailure &&
        error.category === "validator-error" &&
        error.reason === "validator execution failed",
    );
  } finally {
    workspace.cleanup();
  }
});
