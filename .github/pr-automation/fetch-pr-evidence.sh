#!/usr/bin/env bash
# Materializes the untrusted pull request head beside the trusted base checkout and derives the evidence the LLM stages are given.
# It runs without repository credentials and writes nothing back.
#
# The model runtime receives only explicitly allowed files and has no command execution or Git access.
# The diff and manifest produced here are therefore the only authoritative statement of what the pull request changes.
#
# This is shared by every stage because the instruction-file removal below is a security boundary that duplicated scripts would eventually drift from.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: fetch-pr-evidence.sh <head-sha> <base-sha>" >&2
  exit 1
fi

head_sha="$1"
base_sha="$2"
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null

git init pr-head
git -C pr-head config --local core.hooksPath /dev/null
git -C pr-head remote add origin "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY.git"
git -C pr-head fetch --no-tags origin "+$head_sha:refs/remotes/origin/pull-request-head"
git -C pr-head fetch --no-tags origin "+$base_sha:refs/remotes/origin/pull-request-base"
git -C pr-head checkout --detach origin/pull-request-head

# The head tree is contributor-controlled.
# Remove symlinks before granting filesystem-reading tools access so they cannot escape the checkout and disclose runner-local files or environment secrets.
find pr-head -type l -delete

# Agent runtimes may discover instruction files in every directory they read, not just the root of the tree they are given.
# These files are contributor-controlled, so removing them recursively keeps the head tree an evidence-only capability.
find pr-head -depth \
  \( -name CLAUDE.md -o -name CLAUDE.local.md -o -name GEMINI.md -o -name AGENTS.md \
     -o -name .claude -o -name .gemini -o -name .cursor -o -name .cursorrules \) \
  -exec rm -rf {} +
rm -rf pr-head/.github/copilot-instructions.md pr-head/.github/instructions

# Pinning the base avoids races with master while the merge-base comparison excludes base-only changes.
mkdir -p pr-evidence
git -C pr-head diff --no-color --find-renames --name-status \
  origin/pull-request-base...origin/pull-request-head > pr-evidence/changed-files.txt
git -C pr-head diff --no-color --find-renames --unified=3 \
  origin/pull-request-base...origin/pull-request-head > pr-evidence/pull-request.diff

# A single oversized file would otherwise crowd out the rest of the evidence. Oversized pull
# requests are already excluded upstream, so this only guards against pathological single changes.
max_bytes=$((1024 * 1024))
if [ "$(wc -c < pr-evidence/pull-request.diff)" -gt "$max_bytes" ]; then
  head -c "$max_bytes" pr-evidence/pull-request.diff > pr-evidence/pull-request.diff.truncated
  for _ in 1 2 3; do
    if iconv -f UTF-8 -t UTF-8 pr-evidence/pull-request.diff.truncated >/dev/null 2>&1; then
      break
    fi
    truncate -s -1 pr-evidence/pull-request.diff.truncated
  done
  iconv -f UTF-8 -t UTF-8 pr-evidence/pull-request.diff.truncated >/dev/null
  printf '\n[diff truncated at %s bytes; read the full tree in pr-head]\n' "$max_bytes" \
    >> pr-evidence/pull-request.diff.truncated
  mv pr-evidence/pull-request.diff.truncated pr-evidence/pull-request.diff
fi

test -s pr-evidence/changed-files.txt
