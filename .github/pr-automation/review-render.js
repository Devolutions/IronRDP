"use strict";

const MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS = 65_536;
const REVIEW_READY_FOOTER = "Push a commit after addressing these findings. If no code change is needed, you may resolve inline threads and comment `@github-actions review-ready` to request human review.";

const SEVERITY_EMOJI = {
  critical: ":purple_circle:",
  high: ":red_circle:",
  medium: ":orange_circle:",
  low: ":yellow_circle:",
};

// Model output is treated as hostile, so it is neutralized before it reaches a bot-authored
// comment or review. HTML, code spans, mentions, and issue references are defused, and the
// Markdown constructs that would otherwise still render as active links, images, or formatting are
// backslash-escaped so that text such as `[label](https://example.invalid)` stays inert prose.
function escapeMarkdown(value) {
  return String(value).replace(/\\/g, "\\\\")
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;").replace(/'/g, "&#39;").replace(/`/g, "&#96;")
    .replace(/@(?=[\w-])/g, "`@`").replace(/(?<!&)#(?=\d)/g, "`#`")
    .replace(/[[\]()!*_~|]/g, "\\$&");
}

function findingIndicator(finding) {
  return `${finding.severity} ${SEVERITY_EMOJI[finding.severity]}` +
    `${finding.question ? " :question:" : ""}`;
}

function reducedCoverageText(reducedCoverage) {
  return ` optional reviewer${reducedCoverage.length === 1 ? "" : "s"} ` +
    `${reducedCoverage.join(", ")} ${reducedCoverage.length === 1 ? "was" : "were"} unavailable`;
}

function reviewBody(marker, review, reducedCoverage = [], provenancePrefix) {
  const findings = review.findings.filter((finding) => finding.start_line === null).map((finding, index) => {
    return `${index + 1}. **${provenancePrefix(finding.sources)} ${escapeMarkdown(finding.title)}** — ` +
      `${findingIndicator(finding)} — ${escapeMarkdown(finding.path)}\n` +
      `   ${escapeMarkdown(finding.rationale)}`;
  }).join("\n");
  const clean = review.findings.length === 0 ? ":green_circle: " : "";
  const coverage = reducedCoverage.length === 0
    ? ""
    : `\n\nReduced coverage:${reducedCoverageText(reducedCoverage.map(escapeMarkdown))}.`;
  return `${marker}\n\n${clean}${escapeMarkdown(review.summary)}${coverage}${findings ? `\n\n${findings}` : ""}` +
    (review.findings.length ? `\n\n${REVIEW_READY_FOOTER}` : "");
}

function inlineReviewCommentBody(finding, provenancePrefix) {
  return `**${provenancePrefix(finding.sources)} ${escapeMarkdown(finding.title)}** — ` +
    `${findingIndicator(finding)} — ${escapeMarkdown(finding.rationale)}`;
}

module.exports = {
  MAXIMUM_GITHUB_REVIEW_BODY_CHARACTERS, REVIEW_READY_FOOTER, escapeMarkdown, findingIndicator,
  inlineReviewCommentBody, reducedCoverageText, reviewBody,
};
