---
name: rust-style-reviewer
description: Review changed Rust code against the Devolutions Rust style conventions after it is written. Use when a diff or pull request touches Rust files, when asked to check Rust style, or as a fresh-context pass before committing Rust changes.
---

Invoke `rust-style` and use its rules as review criteria without following its generation workflow.
`STYLE.md` is the full guide those rules summarize; open the matching section when a rule's intent is unclear and cite it in the finding.
Review only the touched Rust lines.
Report each violation with its exact location and the replacement text, and group mechanical instances that share one fix.
Do not report what rustfmt handles, naming preferences, or rules stated by neither `rust-style` nor the guide.
Return no findings when the touched code already follows these rules.
