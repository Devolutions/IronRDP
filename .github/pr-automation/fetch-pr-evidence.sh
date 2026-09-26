#!/usr/bin/env bash
# Materializes the untrusted pull request head beside the base checkout and derives the evidence the LLM stages are given.
# It runs without repository credentials and writes nothing back.
#
# The model runtime receives only explicitly allowed files and has no command execution or Git access.
# The diff and manifest produced here are therefore the only authoritative statement of what the pull request changes.
#
# This is shared by every stage because the instruction-file removal below is a security boundary that duplicated scripts would eventually drift from.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: fetch-pr-evidence.sh <head-sha> <base-sha> <max-diff-bytes>" >&2
  exit 1
fi

head_sha="$1"
base_sha="$2"
max_bytes="$3"
case "$max_bytes" in
  1048576) limit_mib=1 ;;
  4194304) limit_mib=4 ;;
  *)
    echo "error: invalid evidence diff limit" >&2
    exit 1
    ;;
esac
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null

git init pr-head
git -C pr-head config --local core.hooksPath /dev/null
git -C pr-head remote add origin "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY.git"
git -C pr-head fetch --no-tags origin "+$head_sha:refs/remotes/origin/pull-request-head"
git -C pr-head fetch --no-tags origin "+$base_sha:refs/remotes/origin/pull-request-base"
git -C pr-head checkout --detach origin/pull-request-head

# Apply the trusted evidence policy instead of contributor-controlled diff attributes.
install -m 600 "$script_dir/evidence-diff-attributes" pr-head/.git/info/attributes

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

# Partial diffs cannot support complete classification or review.
if [ "$(wc -c < pr-evidence/pull-request.diff)" -gt "$max_bytes" ]; then
  reason="pull request diff exceeds the $limit_mib MiB evidence limit"
  printf '%s\n' "$reason" > pr-evidence/failure-reason.txt
  printf 'error: %s\n' "$reason" >&2
  exit 1
fi

test -s pr-evidence/changed-files.txt
