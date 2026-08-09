---
name: docs-writer
description: Write clear, concise technical documentation that helps readers complete a task or understand an interface. Use whenever creating or substantially rewriting READMEs, guides, tutorials, reference docs, rustdoc, explanatory comments, or release notes.
---

# Documentation writer

For Markdown, invoke `markdown-writer`; otherwise invoke `prose-writer`.
Apply the selected writer's rules throughout.
Identify the intended reader, their goal, and the prerequisites they need.
Before restructuring existing documentation, inventory its unique facts so none are lost or restated.
Present the outcome or shortest successful path before internal mechanics and edge cases.
Use only the headings needed to answer distinct reader questions.
Explain one path completely before introducing alternatives, and link to existing material instead of repeating it.
Keep examples minimal, realistic, and responsible for unique information.
Place warnings and constraints beside the step they affect.
Preserve established terminology and verify technical claims against the implementation.

Write the requested documentation directly.
Prefer no net word-count growth when restructuring existing documentation without adding requested information.
New structure should replace or consolidate prose; allow small justified growth when it materially improves navigation without duplication.
Before finishing, remove repetition and confirm that a reader can find the next action without reconstructing it from implementation details.
