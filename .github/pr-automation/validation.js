"use strict";

// Shared strict-validation helpers. Every model output is hostile input: nothing here trusts a
// prototype, an extra key, or a control character.

const SHA = /^[0-9a-f]{40}$/;
const REPO_PATH = /^(?!\/)(?!.*(?:^|\/)\.\.(?:\/|$)).+$/;
const MAXIMUM_GITHUB_INTEGER = 2_147_483_647;
const FORBIDDEN_TEXT_CONTROL = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/;

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

function unicodeLength(value) {
  return value.isWellFormed() ? [...value].length : Number.POSITIVE_INFINITY;
}

function normalizeText(value, maximumLength) {
  if (typeof value !== "string") return null;
  if (!value.isWellFormed() || unicodeLength(value) > maximumLength ||
      FORBIDDEN_TEXT_CONTROL.test(value)) return null;
  // Structured output occasionally represents an empty string as the literal text `""`.
  const normalized = (value === '""' ? "" : value).replace(/\s+/g, " ").trim();
  return normalized;
}

function parseJson(raw) {
  if (typeof raw !== "string") return raw;
  try { return JSON.parse(raw); } catch { return null; }
}

function isBoundedArray(value, maximum) {
  return Array.isArray(value) && value.length <= maximum;
}

function linesAreValidated(path, start, end, changedLines) {
  const lines = changedLines instanceof Map ? changedLines.get(path) : changedLines?.[path];
  const changed = lines instanceof Set ? lines :
    Array.isArray(lines) && lines.every(Number.isSafeInteger) ? new Set(lines) : null;
  if (!changed || end - start >= changed.size) return false;
  for (let line = start; line <= end; line += 1) if (!changed.has(line)) return false;
  return true;
}

module.exports = {
  MAXIMUM_GITHUB_INTEGER, REPO_PATH, SHA,
  exactKeys, invalid, isBoundedArray, isPlainObject, linesAreValidated, normalizeText, parseJson,
  unicodeLength,
};
