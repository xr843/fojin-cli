#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  printf 'usage: %s <archive> <target> <version>\n' "$0" >&2
  exit 2
fi

python3 - "$1" "$2" "$3" <<'PY'
import pathlib
import stat
import sys
import tarfile
import zipfile


archive = pathlib.Path(sys.argv[1])
target = sys.argv[2]
version = sys.argv[3]
top = f"fojin-{version}-{target}"
binary = "fojin.exe" if target.endswith("-windows-msvc") else "fojin"
required_files = {binary, "README.md", "LICENSE-MIT", "LICENSE-APACHE"}
expected = {top, *(f"{top}/{name}" for name in required_files)}


def checked_name(name: str) -> str:
    if "\\" in name:
        raise ValueError(f"archive member uses a backslash: {name}")
    path = pathlib.PurePosixPath(name)
    if path.is_absolute() or not path.parts or any(part in {"", ".", ".."} for part in path.parts):
        raise ValueError(f"unsafe archive member path: {name}")
    return name.rstrip("/")


def validate_members(members):
    seen = set()
    unix_binary_mode = None
    for name, kind, mode in members:
        name = checked_name(name)
        if name in seen:
            raise ValueError(f"duplicate archive member: {name}")
        seen.add(name)
        if name not in expected:
            raise ValueError(f"unexpected archive member: {name}")
        if name == top:
            if kind != "directory":
                raise ValueError(f"top-level member is not a directory: {name}")
        elif kind != "file":
            raise ValueError(f"archive member is not a regular file: {name}")
        if name == f"{top}/{binary}":
            unix_binary_mode = mode

    missing = expected - seen
    if missing:
        raise ValueError("missing archive members: " + ", ".join(sorted(missing)))
    if binary == "fojin" and (unix_binary_mode is None or unix_binary_mode & 0o111 == 0):
        raise ValueError("Unix release binary is not executable")


def tar_members():
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle.getmembers():
            if member.isdir():
                kind = "directory"
            elif member.isfile():
                kind = "file"
            else:
                kind = "other"
            yield member.name, kind, member.mode


def zip_members():
    with zipfile.ZipFile(archive) as bundle:
        for member in bundle.infolist():
            mode = member.external_attr >> 16
            if member.create_system == 3:
                if stat.S_ISLNK(mode):
                    kind = "other"
                elif stat.S_ISDIR(mode):
                    kind = "directory"
                elif stat.S_ISREG(mode):
                    kind = "file"
                else:
                    kind = "other"
            elif member.is_dir():
                kind = "directory"
            else:
                kind = "file"
            yield member.filename, kind, mode


try:
    if archive.name.endswith(".tar.gz"):
        validate_members(tar_members())
    elif archive.suffix == ".zip":
        validate_members(zip_members())
    else:
        raise ValueError(f"unsupported archive extension: {archive.name}")
except (OSError, tarfile.TarError, zipfile.BadZipFile, ValueError) as error:
    print(f"release archive validation failed: {error}", file=sys.stderr)
    raise SystemExit(1)

print(f"release archive validated: {archive.name}")
PY
