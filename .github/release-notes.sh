#!/usr/bin/env bash
# Prints the release notes for a tag: the CHANGELOG.md section of that version, else the tag message.
set -euo pipefail
tag="$1"
version="${tag#v}"
notes=""
if [ -f CHANGELOG.md ]; then
  notes=$(awk -v v="$version" '
    /^## / {
      if (found) exit
      h = $0; gsub(/[\[\]]/, "", h); split(h, a, " ")
      if (a[2] == v) { found = 1; next }
    }
    found' CHANGELOG.md)
fi
if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
  git fetch --force --tags origin >/dev/null 2>&1 || true
  notes=$(git tag -l --format='%(contents)' "$tag" 2>/dev/null || true)
fi
if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
  notes="Orbo $version"
fi
printf '%s\n' "$notes"
