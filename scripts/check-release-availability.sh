#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'usage: %s <owner/repository> <vX.Y.Z tag>\n' "$0" >&2
  exit 2
fi

repository="$1"
tag="$2"

if [[ -z "${GH_TOKEN:-}" ]]; then
  printf 'GH_TOKEN is required to inspect draft and published releases\n' >&2
  exit 1
fi

if ! releases="$(gh api --paginate --slurp "repos/$repository/releases?per_page=100")"; then
  printf 'failed to list existing releases for %s\n' "$repository" >&2
  exit 1
fi

python3 - "$tag" 3<<<"$releases" <<'PY'
import json
import os
import sys


requested_tag = sys.argv[1]
try:
    pages = json.load(os.fdopen(3))
except (OSError, UnicodeError, json.JSONDecodeError) as error:
    print(f"invalid release API JSON: {error}", file=sys.stderr)
    raise SystemExit(1)

if not isinstance(pages, list):
    print("invalid release API JSON: expected an array of pages", file=sys.stderr)
    raise SystemExit(1)

for page in pages:
    if not isinstance(page, list):
        print("invalid release API JSON: expected each page to be an array", file=sys.stderr)
        raise SystemExit(1)
    for release in page:
        if not isinstance(release, dict) or not isinstance(release.get("tag_name"), str):
            print("invalid release API JSON: release lacks a string tag_name", file=sys.stderr)
            raise SystemExit(1)
        if release["tag_name"] == requested_tag:
            print(f"release already exists for tag {requested_tag}", file=sys.stderr)
            raise SystemExit(1)

print(f"no existing release found for tag {requested_tag}")
PY
