---
name: llm-comment-footer
description: Add required LLM attribution to GitHub issue bodies and issue or pull request comments. Use whenever drafting, creating, or replying to this content, including agent-merge review-thread replies and helper-driven GitHub comment replies. Do not use for pull request bodies.
---

# LLM comment footer

Append exactly one of these footers to the issue body or comment Markdown after a blank line:

- `_LLM-assisted comment: auto-replied (no human feedback)._`
- `_LLM-assisted comment: tuned by human._`

Use `auto-replied` only when the comment is posted without human review, edits, or feedback.
Use `tuned by human` when a human reviews, edits, or provides feedback that shapes the final comment.

For app-owned `agent-merge` and other helper-driven replies, treat the helper's app-managed byline as separate from the reply content.
Include one allowed footer in the reply Markdown unless the helper's configured byline exactly matches an allowed footer.
Never silently assume the helper will add the footer.

Do not add the footer to pull request bodies because it conflicts with pull request guidelines.
