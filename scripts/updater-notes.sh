#!/bin/sh
# Print updater notes for an annotated tag.
#
# A shallow GitHub checkout does not fetch the tag object. `git tag -l
# --format=%(contents)` then prints the commit message, which is what shipped
# in the 0.1.10 updater notes. This script fetches the tag and uses the
# annotation. If that is missing, or it is the commit message, it uses
# docs/release-notes/<version>.md. It never prints the commit message.
#
# Usage: scripts/updater-notes.sh <repo> <tag>
set -eu
repo=${1:-.}
tag=${2:-}
cd "$repo"
version=${tag#v}
notes=""
if [ -n "$tag" ]; then
  git fetch --force origin "refs/tags/${tag}:refs/tags/${tag}" >/dev/null 2>&1 || true
  if git cat-file -t "refs/tags/$tag" >/dev/null 2>&1; then
    kind=$(git cat-file -t "refs/tags/$tag")
    if [ "$kind" = "tag" ]; then
      notes=$(git for-each-ref --format='%(contents)' "refs/tags/$tag")
    fi
  fi
fi
commit=$(git log -1 --format=%B 2>/dev/null || true)
trimmed_notes=$(printf '%s' "$notes" | sed -e 's/[[:space:]]*$//')
trimmed_commit=$(printf '%s' "$commit" | sed -e 's/[[:space:]]*$//')
if [ -z "$trimmed_notes" ] || [ "$trimmed_notes" = "$trimmed_commit" ]; then
  file="docs/release-notes/${version}.md"
  if [ -f "$file" ]; then
    notes=$(cat "$file")
  else
    notes="Bolo ${version}"
  fi
fi
printf '%s\n' "$notes" | sed -e 's/[[:space:]]*$//'
