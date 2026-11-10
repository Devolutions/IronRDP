"use strict";

const AI_COUNTS = Object.freeze(["ai-reviewed/1", "ai-reviewed/2", "ai-reviewed/3"]);

function reviewCount(labels) {
  const present = new Set([...labels].map((label) => typeof label === "string" ? label : label?.name));
  const counts = AI_COUNTS.filter((label) => present.has(label));
  return counts.length > 1 ? undefined : counts[0] ?? null;
}

function nextReviewCount(count) {
  return AI_COUNTS[Math.min(AI_COUNTS.indexOf(count) + 1, AI_COUNTS.length - 1)];
}

module.exports = { AI_COUNTS, nextReviewCount, reviewCount };
