#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'usage: %s <vX.Y.Z tag> <package version>\n' "$0" >&2
  exit 2
fi

tag="$1"
package_version="$2"
stable_component='(0|[1-9][0-9]*)'

if [[ ! "$tag" =~ ^v${stable_component}\.${stable_component}\.${stable_component}$ ]]; then
  printf 'release tag must have stable vX.Y.Z form: %s\n' "$tag" >&2
  exit 1
fi

if [[ "$tag" != "v${package_version}" ]]; then
  printf 'release tag %s does not match package version %s\n' "$tag" "$package_version" >&2
  exit 1
fi

printf 'release tag %s matches package version %s\n' "$tag" "$package_version"
