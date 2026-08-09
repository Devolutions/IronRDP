---
name: llm-comment-footer
description: Add required LLM attribution to GitHub issue bodies and issue or pull request comments. Use whenever drafting, creating, or replying to this content, including agent-merge review-thread replies and helper-driven GitHub comment replies. Do not use for pull request bodies.
---

# LLM comment footer

Include exactly one attribution.
Append one of these footers to the issue body or comment Markdown after a blank line unless using a helper-managed byline:

- `> [!NOTE]`
  `> LLM-assisted content (no human feedback).`
- `> [!NOTE]`
  `> Human-tuned, LLM-assisted content.`

Use the no-human-feedback footer only when the content is posted without human review, edits, or feedback.
Use the human-tuned footer when a human reviews, edits, or provides feedback that shapes the final content.

For app-owned `agent-merge` and other helper-driven replies, use an allowed footer in the reply Markdown or the helper's configured app-managed byline.
Ensure the chosen attribution is actually inserted; never assume the helper will add it or include both.

Do not add the footer to pull request bodies because it conflicts with pull request guidelines.
