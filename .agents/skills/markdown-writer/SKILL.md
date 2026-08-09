---
name: markdown-writer
description: Write clean, readable Markdown with concise prose and maintainable source formatting. Use whenever creating or substantially rewriting Markdown files, including READMEs, guides, release notes, issue templates, and Markdown-based documentation.
---

# Markdown writer

Invoke `prose-writer` before drafting and apply its rules throughout.
Use the shallowest heading hierarchy that makes the document easy to scan.
Prefer short paragraphs and lists only when items are easier to compare or follow separately.
Use fenced code blocks with a language identifier and keep explanatory prose outside the fence.

Prefer reference-style links so URLs do not interrupt the source text.
Place link definitions near the end of the document and reuse them for repeated destinations.
Keep inline links when the literal URL is meaningful, the surrounding format requires one, or the link appears in a badge, raw HTML, generated content, or a compact table.

Preserve valid embedded HTML and repository-specific Markdown conventions.
Before finishing, check heading order, code fences, link definitions, and one sentence per source line.
