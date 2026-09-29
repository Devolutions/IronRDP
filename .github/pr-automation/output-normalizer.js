"use strict";

const { hasForbiddenTextControl } = require("./validation");

const CLASSIFIER = require("./schemas/classifier.json");
const SPECIALIST = require("./schemas/candidate-review.json");
const GENERAL = require("./schemas/final-review.json");

const CLASSIFIER_PROSE = new Set([
  "summary",
  "breaking_change_rationale",
  "breaking_change_surface",
]);
const SPECIALIST_PROSE = new Set(["summary"]);
const GENERAL_PROSE = new Set([
  "summary",
  "candidate_dispositions.*.rationale",
]);

function normalizeClassifier(value) {
  return project(value, CLASSIFIER, CLASSIFIER_PROSE);
}

function normalizeSpecialist(value) {
  return project(value, SPECIALIST, SPECIALIST_PROSE);
}

function normalizeGeneral(value) {
  return project(value, GENERAL, GENERAL_PROSE);
}

function project(value, schema, prose, path = []) {
  if (isStrictObjectSchema(schema)) {
    if (!isObject(value)) return value;
    const result = {};
    for (const [key, childSchema] of Object.entries(schema.properties || {})) {
      if (!Object.hasOwn(value, key)) continue;
      const childPath = [...path, key];
      result[key] = normalizeTail(
        project(value[key], childSchema, prose, childPath),
        prose.has(pathKey(childPath)) ? childSchema.maxLength : undefined,
      );
    }
    return result;
  }
  if (schema?.type === "array" && Array.isArray(value)) {
    return value.map((entry) => project(entry, schema.items, prose, [...path, "*"]));
  }
  return value;
}

function isStrictObjectSchema(schema) {
  return schema?.additionalProperties === false && schema.properties &&
    !Array.isArray(schema.type) && schema.type === "object";
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function pathKey(path) {
  return path.join(".");
}

function normalizeTail(value, maximumLength) {
  if (maximumLength === undefined || typeof value !== "string" || hasForbiddenTextControl(value)) {
    return value;
  }
  const normalized = value.replace(/\s+/g, " ").trim();
  const codePoints = [...normalized];
  return codePoints.length <= maximumLength
    ? normalized
    : `${codePoints.slice(0, maximumLength - 1).join("")}…`;
}

module.exports = { normalizeClassifier, normalizeGeneral, normalizeSpecialist };
