"use strict";

// Shared strict-validation helpers. Every model output is hostile input: nothing here trusts a
// prototype, an extra key, or a control character.

const SHA = /^[0-9a-f]{40}$/;
const REPO_PATH = /^(?!\/)(?!.*(?:^|\/)\.\.(?:\/|$)).+$/;

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
      /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/.test(normalized)) return null;
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
  REPO_PATH, SHA,
  exactKeys, invalid, isBoundedArray, isPlainObject, normalizeText, parseJson,
};
