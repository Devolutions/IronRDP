---
name: prose-verifier
description: Verify edited prose is concise, human-first, and written one sentence per source line. Use whenever Markdown, AsciiDoc, plain-text documentation, release notes, or other hand-maintained prose is changed, especially when reviewing documentation style or line-oriented diffs.
---

# Prose verifier

Invoke `prose-writer` and use its rules as review criteria without following its generation workflow.
Review only the touched prose and pay particular attention to concise, human-first phrasing and one sentence per source line.
Report exact locations and replacement text for violations; group mechanical instances that share one fix.
Prefer localized corrections over rewriting compliant surrounding prose.
Return no findings when the touched prose already complies.
