#!/usr/bin/env bash
# Materializes the untrusted pull request head beside the trusted base checkout and derives the
# evidence the LLM stages are given. It runs without repository credentials and writes nothing back.
#
# The stages run claude-code-action in explicit-prompt mode, which supplies only the prompt: no
# changed-file context is injected, and Bash is denied, so the model cannot compute a diff itself.
# The diff and manifest produced here are therefore the only reliable statement of what the pull
# request changes.
#
# This is shared by every stage on purpose. The removal of contributor-controlled instruction files
# below is a security boundary, and three hand-maintained copies of it would eventually drift.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: fetch-pr-evidence.sh <head-sha>" >&2
  exit 1
fi

head_sha="$1"
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null

git init pr-head
git -C pr-head config --local core.hooksPath /dev/null
git -C pr-head remote add origin "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY.git"
git -C pr-head fetch --no-tags origin "+$head_sha:refs/remotes/origin/pull-request-head"
git -C pr-head fetch --no-tags origin +master:refs/remotes/origin/master
git -C pr-head checkout --detach origin/pull-request-head

# Claude Code discovers agent instruction files in every directory it reads, not just the root of
# the tree it is given. These files are contributor-controlled here, so a nested pr-head/sub/CLAUDE.md
# or pr-head/sub/.claude would otherwise escape the evidence-only boundary. The removal is recursive.
find pr-head -depth \
  \( -name CLAUDE.md -o -name CLAUDE.local.md -o -name AGENTS.md \
     -o -name .claude -o -name .cursor -o -name .cursorrules \) \
  -exec rm -rf {} +
rm -rf pr-head/.github/copilot-instructions.md pr-head/.github/instructions

# Compared against the merge base rather than the base tip so that unrelated commits landing on
# master while the pull request is open are not attributed to it.
base_sha="$(git -C pr-head merge-base origin/master origin/pull-request-head)"
mkdir -p pr-evidence
git -C pr-head diff --no-color --find-renames --name-status \
  "$base_sha" origin/pull-request-head > pr-evidence/changed-files.txt
git -C pr-head diff --no-color --find-renames --unified=3 \
  "$base_sha" origin/pull-request-head > pr-evidence/pull-request.diff

# A single oversized file would otherwise crowd out the rest of the evidence. Oversized pull
# requests are already excluded upstream, so this only guards against pathological single changes.
max_bytes=$((1024 * 1024))
if [ "$(wc -c < pr-evidence/pull-request.diff)" -gt "$max_bytes" ]; then
  head -c "$max_bytes" pr-evidence/pull-request.diff > pr-evidence/pull-request.diff.truncated
  printf '\n[diff truncated at %s bytes; read the full tree in pr-head]\n' "$max_bytes" \
    >> pr-evidence/pull-request.diff.truncated
  mv pr-evidence/pull-request.diff.truncated pr-evidence/pull-request.diff
fi

test -s pr-evidence/changed-files.txt
