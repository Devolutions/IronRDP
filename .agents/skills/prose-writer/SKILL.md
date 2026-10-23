---
name: prose-writer
description: Write concise, human-first prose with one sentence per source line. Use whenever generating or substantially rewriting Markdown, AsciiDoc, plain-text documentation, code comments, doc comments such as rustdoc, release notes, user-facing explanations, or other hand-maintained prose.
---

# Prose writer

Lead with what the reader needs to know or do.
Use direct sentences, concrete verbs, familiar words, and the shortest phrasing that preserves meaning.
This is plain language at the sentence level: a reader understands on first reading.
Remove throat-clearing, repetition, inflated wording, and details already clear from context.
Keep each paragraph focused on one idea and order ideas by reader need rather than implementation history.
Preserve necessary nuance, technical terms, warnings, and the document's established voice.
A defined term stays where a common word would be less exact, and the same word names the same thing throughout.

In source formats where newlines render as spaces, start each sentence on its own line and do not hard-wrap a sentence.
Code comments and doc comments follow the same convention, except that a sentence wraps only where it would exceed the line-length limit.
Do not apply that convention to code blocks, tables, headings, link definitions, generated files, or formats where line breaks affect rendering.

Write the requested prose directly instead of discussing these rules unless the user asks.
Prefer no net word-count growth when rewriting existing prose without adding requested information.
Accept small growth only when it materially improves clarity without duplicating information.
Treat source line breaks as formatting, not justification for growth.
Before finishing, reread from the reader's perspective and remove anything that does not help them understand or act.
