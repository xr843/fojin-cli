#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version_check="$repo_root/scripts/check-release-version.sh"
archive_check="$repo_root/scripts/check-release-archive.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

expect_pass() {
  local name="$1"
  shift
  if ! "$@" >"$tmp/output" 2>&1; then
    cat "$tmp/output" >&2
    fail "$name unexpectedly failed"
  fi
}

expect_fail() {
  local name="$1"
  shift
  if "$@" >"$tmp/output" 2>&1; then
    fail "$name unexpectedly passed"
  fi
}

expect_pass "matching stable release tag" "$version_check" v1.2.3 1.2.3
expect_fail "mismatched release tag" "$version_check" v1.2.4 1.2.3
expect_fail "missing v prefix" "$version_check" 1.2.3 1.2.3
expect_fail "prerelease tag" "$version_check" v1.2.3-rc.1 1.2.3-rc.1
expect_fail "non-numeric tag" "$version_check" v1.2.x 1.2.x

target="x86_64-unknown-linux-gnu"
version="1.2.3"
staging="fojin-${version}-${target}"
mkdir -p "$tmp/$staging"
printf '#!/bin/sh\nexit 0\n' >"$tmp/$staging/fojin"
chmod 755 "$tmp/$staging/fojin"
printf 'readme\n' >"$tmp/$staging/README.md"
printf 'MIT\n' >"$tmp/$staging/LICENSE-MIT"
printf 'Apache\n' >"$tmp/$staging/LICENSE-APACHE"
tar -C "$tmp" -czf "$tmp/valid.tar.gz" "$staging"
expect_pass "valid Unix archive" "$archive_check" "$tmp/valid.tar.gz" "$target" "$version"

printf 'extra\n' >"$tmp/$staging/EXTRA"
tar -C "$tmp" -czf "$tmp/extra.tar.gz" "$staging"
expect_fail "archive with an extra file" "$archive_check" "$tmp/extra.tar.gz" "$target" "$version"
rm "$tmp/$staging/EXTRA"

rm "$tmp/$staging/LICENSE-APACHE"
tar -C "$tmp" -czf "$tmp/missing.tar.gz" "$staging"
expect_fail "archive missing a license" "$archive_check" "$tmp/missing.tar.gz" "$target" "$version"
printf 'Apache\n' >"$tmp/$staging/LICENSE-APACHE"

chmod 644 "$tmp/$staging/fojin"
tar -C "$tmp" -czf "$tmp/not-executable.tar.gz" "$staging"
expect_fail "archive with non-executable Unix binary" "$archive_check" "$tmp/not-executable.tar.gz" "$target" "$version"
chmod 755 "$tmp/$staging/fojin"

rm "$tmp/$staging/README.md"
ln -s LICENSE-MIT "$tmp/$staging/README.md"
tar -C "$tmp" -czf "$tmp/symlink.tar.gz" "$staging"
expect_fail "archive containing a symlink" "$archive_check" "$tmp/symlink.tar.gz" "$target" "$version"
rm "$tmp/$staging/README.md"
printf 'readme\n' >"$tmp/$staging/README.md"

windows_target="x86_64-pc-windows-msvc"
windows_staging="fojin-${version}-${windows_target}"
mkdir -p "$tmp/$windows_staging"
printf 'binary\n' >"$tmp/$windows_staging/fojin.exe"
printf 'readme\n' >"$tmp/$windows_staging/README.md"
printf 'MIT\n' >"$tmp/$windows_staging/LICENSE-MIT"
printf 'Apache\n' >"$tmp/$windows_staging/LICENSE-APACHE"
python3 -c 'import shutil, sys; shutil.make_archive(sys.argv[2], "zip", root_dir=sys.argv[1], base_dir=sys.argv[3])' \
  "$tmp" "$tmp/valid" "$windows_staging"
expect_pass "valid Windows archive" "$archive_check" "$tmp/valid.zip" "$windows_target" "$version"

python3 - "$tmp/symlink-directory.zip" "$tmp/fifo-file.zip" "$windows_staging" <<'PY'
import stat
import sys
import zipfile


symlink_archive, fifo_archive, staging = sys.argv[1:]
files = ("fojin.exe", "README.md", "LICENSE-MIT", "LICENSE-APACHE")


def add_member(bundle, name, mode, data=b"fixture\n"):
    member = zipfile.ZipInfo(name)
    member.create_system = 3
    member.external_attr = mode << 16
    bundle.writestr(member, data)


with zipfile.ZipFile(symlink_archive, "w") as bundle:
    add_member(bundle, f"{staging}/", stat.S_IFLNK | 0o777, b"elsewhere")
    for name in files:
        add_member(bundle, f"{staging}/{name}", stat.S_IFREG | 0o644)

with zipfile.ZipFile(fifo_archive, "w") as bundle:
    add_member(bundle, f"{staging}/", stat.S_IFDIR | 0o755, b"")
    for name in files:
        mode = stat.S_IFIFO | 0o644 if name == "README.md" else stat.S_IFREG | 0o644
        add_member(bundle, f"{staging}/{name}", mode)
PY
expect_fail "ZIP directory-shaped symlink" "$archive_check" "$tmp/symlink-directory.zip" "$windows_target" "$version"
expect_fail "ZIP expected file encoded as FIFO" "$archive_check" "$tmp/fifo-file.zip" "$windows_target" "$version"

(cd "$tmp" && sha256sum valid.tar.gz valid.zip >SHA256SUMS)
expect_pass "generated checksums" sh -c "cd '$tmp' && sha256sum -c SHA256SUMS"
printf 'tampered\n' >>"$tmp/valid.zip"
expect_fail "tampered archive checksum" sh -c "cd '$tmp' && sha256sum -c SHA256SUMS"

expect_pass "release publication refuses asset overwrite" python3 - "$repo_root/.github/workflows/release.yml" <<'PY'
from pathlib import Path
import sys


def refuses_overwrite(workflow):
    lines = workflow.splitlines()
    uses_index = next(
        index for index, line in enumerate(lines) if "uses: softprops/action-gh-release@" in line
    )
    uses_indent = len(lines[uses_index]) - len(lines[uses_index].lstrip())
    step = []
    for line in lines[uses_index + 1 :]:
        indent = len(line) - len(line.lstrip())
        if line.strip() and indent <= uses_indent and line.lstrip().startswith("-"):
            break
        step.append(line.strip())
    values = [line.partition(":")[2].strip() for line in step if line.startswith("overwrite_files:")]
    return values == ["false"]


workflow = Path(sys.argv[1]).read_text()
setting = "          overwrite_files: false\n"
if workflow.count(setting) != 1 or not refuses_overwrite(workflow):
    raise SystemExit("softprops release step must set overwrite_files: false exactly once")
if refuses_overwrite(workflow.replace(setting, "", 1)):
    raise SystemExit("missing overwrite_files setting was accepted")
if refuses_overwrite(workflow.replace("overwrite_files: false", "overwrite_files: true", 1)):
    raise SystemExit("overwrite_files: true was accepted")
PY

printf 'release script checks passed\n'
