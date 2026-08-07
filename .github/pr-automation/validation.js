"use strict";

// Shared strict-validation helpers. Every model output is hostile input: nothing here trusts a
// prototype, an extra key, a control character, or an embedded instruction.

const SHA = /^[0-9a-f]{40}$/;
const REPO_PATH = /^(?!\/)(?!.*(?:^|\/)\.\.(?:\/|$)).+$/;
// Only instruction-shaped text is rejected. Bare verbs such as "run" are ordinary protocol and Rust
// prose ("run-length encoding", "the tests you can run"), and rejecting them invalidated whole
// model outputs. Model text is length-bounded, control-character free, and Markdown-escaped before
// publication, and it is never fed back to a model as instructions.
const COMMAND_OR_INSTRUCTION = /(?:^|\s)(?:ignore|disregard|override|forget|follow)\b.{0,160}\b(?:instruction|prompt|message|rule|guideline)|(?:system|developer|assistant)\s+(?:message|prompt)|<\/?(?:system|instructions)>/i;

function invalid(reason) {
  return { ok: false, status: "unavailable", reason };
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value) &&
    (Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null);
}

function exactKeys(value, keys) {
  return isPlainObject(value) && Object.keys(value).length === keys.length &&
    keys.every((key) => Object.hasOwn(value, key));
}

function normalizeText(value, maximum) {
  if (typeof value !== "string") return null;
  // Structured output occasionally represents an empty string as the literal text `""`.
  const normalized = (value === '""' ? "" : value).replace(/\s+/g, " ").trim();
  if (Buffer.byteLength(normalized, "utf8") > maximum ||
      /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/.test(normalized) ||
      COMMAND_OR_INSTRUCTION.test(normalized)) return null;
  return normalized;
}

function parseJson(raw, maximumBytes) {
  if (typeof raw !== "string") return raw;
  if (Buffer.byteLength(raw, "utf8") > maximumBytes) return null;
  try { return JSON.parse(raw); } catch { return null; }
}

function isBoundedArray(value, maximum) {
  return Array.isArray(value) && value.length <= maximum;
}

module.exports = {
  COMMAND_OR_INSTRUCTION, REPO_PATH, SHA,
  exactKeys, invalid, isBoundedArray, isPlainObject, normalizeText, parseJson,
};
