---
name: prose-verifier
description: Verify edited prose is concise, human-first, and written one sentence per source line. Use whenever Markdown, AsciiDoc, plain-text documentation, release notes, or other hand-maintained prose is changed, especially when reviewing documentation style or line-oriented diffs.
---

# Prose verifier

Apply one sentence per line to prose source: each sentence starts on its own line, and a sentence is not hard-wrapped across lines.
This keeps source diffs local without changing normal rendered paragraphs.

Also verify that the prose leads with the reader's need, uses direct language, removes filler, and keeps sentences as short as clarity allows.
Preserve necessary nuance, technical terms, and the document's established voice.

Ignore code blocks, tables, headings, link definitions, generated files, and formats where line breaks change rendering or are controlled by a formatter.
Do not force sentence fragments such as list-item labels onto separate lines.
Report exact locations and replacement text for violations; group mechanical instances that share one fix.
Return no findings when the touched prose already complies.
