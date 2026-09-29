"use strict";

const path = require("node:path");

const { fail } = require("./errors");
const { MAX_VALIDATION_REASON_BYTES, MAX_VALIDATOR_METADATA_BYTES } = require("./limits");
const { WorkspaceSandbox } = require("./sandbox");

const SAFE_EXPORT = /^[A-Za-z_$][A-Za-z0-9_$]{0,127}$/;
const SAFE_REASON = /^[A-Za-z0-9][A-Za-z0-9 .,:;()/_-]{0,511}$/;

class ValidatorFailure extends Error {
  constructor(reason, category) {
    super(reason);
    this.name = "ValidatorFailure";
    this.reason = reason;
    this.category = category;
  }
}

function parseMetadata(raw) {
  if (raw === "") return {};
  if (Buffer.byteLength(raw, "utf8") > MAX_VALIDATOR_METADATA_BYTES) {
    fail("validator metadata exceeds byte limit", "input");
  }
  let metadata;
  try {
    metadata = JSON.parse(raw);
  } catch {
    fail("validator metadata is not valid JSON", "input");
  }
  if (metadata === null || typeof metadata !== "object" || Array.isArray(metadata)) {
    fail("validator metadata must be an object", "input");
  }
  return metadata;
}

function resolveSelector(workspace, selector, kind) {
  const marker = selector.lastIndexOf("#");
  if (marker <= 0 || marker === selector.length - 1) {
    fail(`invalid ${kind} selector`, "input");
  }
  const modulePath = selector.slice(0, marker);
  const exportName = selector.slice(marker + 1);
  if (!SAFE_EXPORT.test(exportName)) fail(`invalid ${kind} selector`, "input");

  const sandbox = new WorkspaceSandbox(workspace);
  let target;
  try {
    target = sandbox.resolve(modulePath, "file", false);
  } catch (error) {
    if (kind === "normalizer") {
      throw new ValidatorFailure("normalizer module could not be loaded", "normalizer-error");
    }
    throw error;
  }
  let exports;
  try {
    delete require.cache[require.resolve(target.real)];
    exports = require(target.real);
  } catch {
    throw new ValidatorFailure(`${kind} module could not be loaded`, `${kind}-error`);
  }
  const callback = exports?.[exportName];
  if (typeof callback !== "function") {
    throw new ValidatorFailure(`${kind} export is unavailable`, `${kind}-error`);
  }
  return callback;
}

function loadValidator(workspace, selector, metadata) {
  if (selector === "") return null;
  const validate = resolveSelector(workspace, selector, "validator");
  return async (candidate, context) => {
    try {
      const result = await validate(candidate, {
        metadata,
        previousCandidate: context.previousCandidate,
        candidates: context.candidates,
        repairAttempt: context.repairAttempt,
      });
      if (result === null || typeof result !== "object" || Array.isArray(result) ||
          typeof result.ok !== "boolean") {
        throw new ValidatorFailure("validator returned an invalid result", "validator-error");
      }
      if (result.ok) return { ok: true };
      if (!safeReason(result.reason)) {
        throw new ValidatorFailure("validator returned an unsafe rejection reason", "validator-error");
      }
      return { ok: false, reason: result.reason };
    } catch (error) {
      if (error instanceof ValidatorFailure) throw error;
      throw terminalValidatorFailure(error);
    }
  };
}

function loadNormalizer(workspace, selector) {
  if (selector === "") return null;
  const normalize = resolveSelector(workspace, selector, "normalizer");
  return (candidate) => {
    try {
      return normalize(candidate);
    } catch {
      throw new ValidatorFailure("normalizer execution failed", "normalizer-error");
    }
  };
}

function safeReason(reason) {
  return typeof reason === "string" &&
    Buffer.byteLength(reason, "utf8") <= MAX_VALIDATION_REASON_BYTES &&
    SAFE_REASON.test(reason);
}

function terminalValidatorFailure(error) {
  if (error?.code === "VALIDATOR_TERMINAL") {
    return new ValidatorFailure(
      safeReason(error.message) ? error.message : "validator terminal failure",
      "validator-terminal",
    );
  }
  return new ValidatorFailure("validator execution failed", "validator-error");
}

module.exports = { ValidatorFailure, loadNormalizer, loadValidator, parseMetadata };
