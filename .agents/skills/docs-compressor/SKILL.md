---
name: docs-compressor
description: Review documentation changes for concise, human-readable prose without losing technical meaning. Use whenever READMEs, guides, reference docs, rustdoc, explanatory comments, release notes, or other instructional prose is added or edited.
---

# Documentation compressor

Invoke `docs-writer` and `prose-writer`, then use their rules as review criteria without following their generation workflows.
Challenge every paragraph, heading, example, and aside that does not add information or help the reader act.
Look for the same meaning expressed with less structure, repetition, or wording while preserving the writer rules.
For substantial rewrites, compare word counts before and after.
Prefer no net growth when the task adds no information.
Accept small growth only when it materially improves clarity or navigation without duplicating information, and explain why.

Report only material readability gains.
For each finding, cite the location, explain what obstructs the reader, and provide a concise replacement or a precise deletion.
Combine repeated edits into one finding when the same rewrite addresses them.
