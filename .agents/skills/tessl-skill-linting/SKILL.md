---
name: tessl-skill-linting
description: Review, lint, create, edit, refine, optimize, compress, or validate any agent skill in this repository, including any SKILL.md. Use this skill whenever a user asks to author or change a skill, improve its instructions or trigger coverage, assess skill quality, or reduce its context usage, even when Tessl is not mentioned. Keep skills small and split distinct workflows into focused micro-skills when that reduces context and trigger overlap. Do not use it for ordinary application code, documentation, or workflow changes that are not agent skills.
---

# Tessl Skill Linting

Keep repository-local agent skills correct, focused, and inexpensive to load.

## Workflow

1. Identify the relevant skill directories under `.agents\skills`.
   For an existing skill, read its `SKILL.md` before editing and preserve its intended behavior and scope.
   For a new skill, inspect neighboring skills for conventions, then create the target directory and initial `SKILL.md` from the requested scope.
2. Inspect the frontmatter and instructions for valid structure, clear trigger conditions, focused scope, missing prerequisites, accurate and safe tooling steps, ambiguity, brittleness, redundancy, and likely failure cases.
3. Run local structural validation for every relevant skill:

   ```powershell
   pwsh <this-skill-directory>\scripts\Test-TesslSkill.ps1 -SkillDirectory <skill-directory>
   ```

   The helper stages the skill in a temporary directory, creates Tessl's required local metadata there, runs `tessl skill lint <temporary-skill-directory>`, and removes the staging directory.
   Treat lint findings as input to fix when appropriate.
   Tessl lint validates structure, not whether the workflow is semantically complete.
4. Make only clearly justified edits inside the relevant skill and directly related local eval artifacts.
   Do not change application code, unrelated skills, or repository configuration.
5. Perform a deliberate compression pass.
   Remove filler, repetition, ceremonial headings, generic advice, redundant rules, needless examples, and details better suited to a progressive-disclosure reference.
   Prefer concise imperative language.
   Keep the initial `SKILL.md` minimal; add a reference only when specialized material is substantial and needed conditionally.
   Prefer focused micro-skills over one broad skill when distinct workflows can trigger or load independently.
6. Re-run the local lint helper after edits and inspect the final diff.

## Report

State:

- Tessl lint findings and their resolution.
- Meaningful edits and the behavior they preserve or improve.
- Two or three recommended future local test prompts that exercise triggering and instructions.

Use Tessl only for temporary local import and structural linting.
Cloud features, including `tessl skill review`, managed evals, scenarios, project creation or linking, publishing, and credentials, are out of scope.
