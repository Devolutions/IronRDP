---
name: docs-compressor
description: Review documentation changes for concise, human-readable prose without losing technical meaning. Use whenever READMEs, guides, reference docs, rustdoc, explanatory comments, release notes, or other instructional prose is added or edited.
---

# Documentation compressor

Make the shortest version that remains accurate and useful to its intended reader.
Remove repetition, throat-clearing, redundant headings, inflated wording, unnecessary examples, and details already clear from nearby context.
Prefer direct sentences, concrete verbs, familiar words, and a structure that presents the reader's goal before implementation detail.

Preserve technical precision, prerequisites, warnings, terminology, links, examples that carry unique information, and distinctions that affect behavior.
Do not compress prose into fragments, erase useful rationale, or replace established project vocabulary merely to shorten it.

Report only material readability gains.
For each finding, cite the location, explain what obstructs the reader, and provide a concise replacement or a precise deletion.
Combine repeated edits into one finding when the same rewrite addresses them.
