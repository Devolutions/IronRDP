#!/usr/bin/env bash
# Stages the Open Specifications corpus as inert review data from an exact checked-out commit.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: prepare-review-sources.sh <openspecs-repository> <commit-sha>" >&2
  exit 1
fi

repository="$1"
expected_sha="$2"
corpus_prefix="skills/windows-protocols/"
destination_parent="review-sources"
destination="$destination_parent/windows-protocols"
staging="$destination_parent/.windows-protocols.$$"

if [[ ! "$expected_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "invalid corpus commit" >&2
  exit 1
fi
if [ ! -d "$repository/.git" ] ||
   [ "$(git -C "$repository" rev-parse --verify HEAD)" != "$expected_sha" ] ||
   [ "$(git -C "$repository" rev-parse --verify "$expected_sha^{commit}")" != "$expected_sha" ]; then
  echo "corpus checkout does not match the expected commit" >&2
  exit 1
fi
if [ -L "$destination_parent" ] || [ -L "$destination" ]; then
  echo "review source destination must not be a symlink" >&2
  exit 1
fi

mkdir -p "$destination_parent"
rm -rf "$staging"
mkdir "$staging"
trap 'rm -rf "$staging"' EXIT

copied=0
while IFS= read -r -d '' entry; do
  metadata="${entry%%$'\t'*}"
  corpus_path="${entry#*$'\t'}"
  read -r mode type object <<< "$metadata"

  if [[ "$corpus_path" != "$corpus_prefix"* ]] ||
     [[ "/$corpus_path/" == *"/../"* ]] ||
     [[ "/$corpus_path/" == *"/./"* ]] ||
     [[ "$corpus_path" == /* ]] ||
     [[ "$corpus_path" == *\\* ]]; then
    echo "unsafe corpus path" >&2
    exit 1
  fi
  if [ "$type" != "blob" ] || [ "$mode" = "120000" ] || [ "$mode" = "160000" ]; then
    echo "corpus contains a symlink, submodule, or non-file entry" >&2
    exit 1
  fi

  relative="${corpus_path#"$corpus_prefix"}"
  include=false
  if [ "$relative" = "README.md" ] || [ "$relative" = "LEGAL.md" ]; then
    include=true
  elif [[ "$relative" == */* ]]; then
    protocol_id="${relative%%/*}"
    protocol_file="${relative#*/}"
    if [[ "$protocol_id" =~ ^(MS|MC)-[A-Z0-9]+$ ]] &&
       [ "$protocol_file" = "$protocol_id.md" ]; then
      include=true
    fi
  fi

  if [ "$include" = true ]; then
    if [ "$mode" != "100644" ]; then
      echo "corpus markdown must be a regular non-executable file" >&2
      exit 1
    fi
    mkdir -p "$staging/$(dirname "$relative")"
    git -C "$repository" cat-file blob "$object" > "$staging/$relative"
    copied=$((copied + 1))
  fi
done < <(git -C "$repository" ls-tree -r -z "$expected_sha" -- "${corpus_prefix%/}")

if [ "$copied" -eq 0 ] || [ ! -f "$staging/README.md" ]; then
  echo "corpus contains no expected markdown data" >&2
  exit 1
fi

printf '%s\n' "$expected_sha" > "$staging/.corpus-commit"
rm -rf "$destination"
mv "$staging" "$destination"
trap - EXIT
