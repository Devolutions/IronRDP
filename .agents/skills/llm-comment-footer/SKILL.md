---
name: llm-comment-footer
description: Add an LLM-usage footer to comments on GitHub issues and pull requests. Use whenever drafting, creating, or replying to an issue comment, pull request conversation comment, or pull request review comment. Do not use for issue bodies or pull request bodies.
---

# LLM comment footer

Append exactly one of these footers after a blank line:

- `_LLM-assisted comment: auto-replied (no human feedback)._`
- `_LLM-assisted comment: tuned by human._`

Use `auto-replied` only when the comment is posted without human review, edits, or feedback.
Use `tuned by human` when a human reviews, edits, or provides feedback that shapes the final comment.
Do not add the footer to issue bodies, pull request bodies, commit messages, or other content.
