#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
installer="$repo_root/install.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

real_awk="$(command -v awk)"
real_cp="$(command -v cp)"
real_gzip="$(command -v gzip)"
real_install="$(command -v install)"
real_mkdir="$(command -v mkdir)"
real_mktemp="$(command -v mktemp)"
real_rm="$(command -v rm)"
real_sh="$(command -v sh)"
real_shasum="$(command -v shasum || true)"
real_sha256sum="$(command -v sha256sum || true)"
real_tar="$(command -v tar)"

version="v9.8.7"
target="x86_64-unknown-linux-gnu"
asset="fojin-${version#v}-${target}.tar.gz"
release_base="https://github.com/xr843/fojin-cli/releases/download/$version"
fixture_root="$tmp/fixture/fojin-${version#v}-${target}"
fixture_archive="$tmp/$asset"
mkdir -p "$fixture_root"
cat >"$fixture_root/fojin" <<'EOF'
#!/bin/sh
printf 'fojin 9.8.7\n'
EOF
chmod 755 "$fixture_root/fojin"
tar -C "$tmp/fixture" -czf "$fixture_archive" "fojin-${version#v}-${target}"

if [[ -n "$real_sha256sum" ]]; then
  digest="$($real_sha256sum "$fixture_archive" | awk '{ print $1 }')"
elif [[ -n "$real_shasum" ]]; then
  digest="$($real_shasum -a 256 "$fixture_archive" | awk '{ print $1 }')"
else
  printf 'FAIL: test host needs sha256sum or shasum\n' >&2
  exit 1
fi

failures=0

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  failures=$((failures + 1))
}

write_common_tools() {
  local bin="$1"

  mkdir -p "$bin"
  ln -s "$real_awk" "$bin/awk"
  ln -s "$real_gzip" "$bin/gzip"
  ln -s "$real_mkdir" "$bin/mkdir"
  ln -s "$real_mktemp" "$bin/mktemp"
  ln -s "$real_rm" "$bin/rm"

  cat >"$bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) printf 'Linux\n' ;;
  -m) printf 'x86_64\n' ;;
  *) exit 1 ;;
esac
EOF

  cat >"$bin/curl" <<'EOF'
#!/bin/sh
set -eu
url=
output=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      shift
      output="$1"
      ;;
    https://*) url="$1" ;;
  esac
  shift
done
[ -n "$url" ] && [ -n "$output" ] || exit 2
printf '%s\n' "$url" >>"$CURL_LOG"
case "$url" in
  */"$ASSET") "$REAL_CP" "$FIXTURE_ARCHIVE" "$output" ;;
  */SHA256SUMS)
    [ "$CHECKSUM_MODE" != "download-failure" ] || exit 22
    "$REAL_CP" "$CHECKSUM_FIXTURE" "$output"
    ;;
  *) exit 22 ;;
esac
EOF

  cat >"$bin/tar" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"$TAR_LOG"
exec "$REAL_TAR" "$@"
EOF

  cat >"$bin/install" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"$INSTALL_LOG"
exec "$REAL_INSTALL" "$@"
EOF

  chmod +x "$bin/curl" "$bin/install" "$bin/tar" "$bin/uname"
}

write_hash_tool() {
  local bin="$1"
  local kind="$2"

  case "$kind" in
    sha256sum)
      [[ -n "$real_sha256sum" ]] || return 1
      cat >"$bin/sha256sum" <<'EOF'
#!/bin/sh
printf 'sha256sum %s\n' "$*" >>"$HASH_LOG"
PATH=/usr/bin:/bin exec "$REAL_SHA256SUM" "$@"
EOF
      chmod +x "$bin/sha256sum"
      ;;
    shasum)
      [[ -n "$real_shasum" ]] || return 1
      cat >"$bin/shasum" <<'EOF'
#!/bin/sh
printf 'shasum %s\n' "$*" >>"$HASH_LOG"
PATH=/usr/bin:/bin exec "$REAL_SHASUM" "$@"
EOF
      chmod +x "$bin/shasum"
      ;;
    hash-extra-field)
      cat >"$bin/sha256sum" <<'EOF'
#!/bin/sh
printf 'sha256sum %s\n' "$*" >>"$HASH_LOG"
printf '%s  - unexpected\n' "$EXPECTED_DIGEST"
EOF
      chmod +x "$bin/sha256sum"
      ;;
    hash-extra-line)
      cat >"$bin/sha256sum" <<'EOF'
#!/bin/sh
printf 'sha256sum %s\n' "$*" >>"$HASH_LOG"
printf '%s  -\nunexpected output\n' "$EXPECTED_DIGEST"
EOF
      chmod +x "$bin/sha256sum"
      ;;
    none) : ;;
    *) return 1 ;;
  esac
}

write_checksum_fixture() {
  local scenario="$1"
  local destination="$2"
  local short_digest="${digest%?}"
  local non_hex_digest="${digest%?}z"

  case "$scenario" in
    valid)
      printf '%s  unrelated-one.tar.gz\n%s  %s\n%s  unrelated-two.zip\n' \
        "$digest" "$digest" "$asset" "$digest" >"$destination"
      ;;
    missing-record) printf '%s  unrelated.tar.gz\n' "$digest" >"$destination" ;;
    duplicate-record)
      printf '%s  %s\n%s  %s\n' "$digest" "$asset" "$digest" "$asset" >"$destination"
      ;;
    valid-plus-marker-target)
      printf '%s  %s\n%s marker %s\n' "$digest" "$asset" "$digest" "$asset" >"$destination"
      ;;
    valid-plus-star-target)
      printf '%s  %s\n%s  *%s\n' "$digest" "$asset" "$digest" "$asset" >"$destination"
      ;;
    malformed-digest) printf '%s  %s\n' "$short_digest" "$asset" >"$destination" ;;
    non-hex-digest) printf '%s  %s\n' "$non_hex_digest" "$asset" >"$destination" ;;
    extra-field) printf '%s  %s unexpected\n' "$digest" "$asset" >"$destination" ;;
    wrong-digest) printf '%064d  %s\n' 0 "$asset" >"$destination" ;;
    download-failure) : >"$destination" ;;
    *) return 1 ;;
  esac
}

prepare_case() {
  local name="$1"
  local scenario="$2"
  local hash_kind="$3"
  local case_dir="$tmp/cases/$name"

  mkdir -p "$case_dir/home" "$case_dir/install"
  : >"$case_dir/curl.log"
  : >"$case_dir/hash.log"
  : >"$case_dir/install.log"
  : >"$case_dir/tar.log"
  write_checksum_fixture "$scenario" "$case_dir/SHA256SUMS"
  write_common_tools "$case_dir/bin"
  write_hash_tool "$case_dir/bin" "$hash_kind"
  printf '%s\n' "$case_dir"
}

run_installer() {
  local case_dir="$1"
  local scenario="$2"

  PATH="$case_dir/bin" \
  HOME="$case_dir/home" \
  FOJIN_INSTALL_DIR="$case_dir/install" \
  FOJIN_VERSION="$version" \
  ASSET="$asset" \
  CHECKSUM_FIXTURE="$case_dir/SHA256SUMS" \
  CHECKSUM_MODE="$scenario" \
  CURL_LOG="$case_dir/curl.log" \
  HASH_LOG="$case_dir/hash.log" \
  INSTALL_LOG="$case_dir/install.log" \
  TAR_LOG="$case_dir/tar.log" \
  FIXTURE_ARCHIVE="$fixture_archive" \
  REAL_CP="$real_cp" \
  REAL_INSTALL="$real_install" \
  REAL_SHA256SUM="$real_sha256sum" \
  REAL_SHASUM="$real_shasum" \
  REAL_TAR="$real_tar" \
  EXPECTED_DIGEST="$digest" \
  "$real_sh" "$installer"
}

assert_release_urls() {
  local name="$1"
  local curl_log="$2"
  local expected="$release_base/$asset
$release_base/SHA256SUMS"
  local actual
  actual="$(cat "$curl_log")"
  [[ "$actual" == "$expected" ]] || fail "$name did not download archive and SHA256SUMS from the same release"
}

expect_success() {
  local name="$1"
  local hash_kind="$2"
  local failures_before="$failures"
  local case_dir
  case_dir="$(prepare_case "$name" valid "$hash_kind")" || {
    fail "$name could not prepare $hash_kind fixture"
    return
  }

  if ! run_installer "$case_dir" valid >"$case_dir/output" 2>&1; then
    cat "$case_dir/output" >&2
    fail "$name unexpectedly failed"
    return
  fi
  assert_release_urls "$name" "$case_dir/curl.log"
  [[ "$(wc -l <"$case_dir/tar.log")" -eq 1 ]] || fail "$name did not extract exactly once"
  [[ "$(wc -l <"$case_dir/install.log")" -eq 1 ]] || fail "$name did not invoke install exactly once"
  grep -q "^$hash_kind " "$case_dir/hash.log" || fail "$name did not use $hash_kind"
  [[ -x "$case_dir/install/fojin" ]] || fail "$name did not install the binary"
  if [[ "$failures" -eq "$failures_before" ]]; then
    printf 'PASS: %s\n' "$name"
  fi
}

expect_failure_before_extract() {
  local name="$1"
  local scenario="$2"
  local hash_kind="${3:-sha256sum}"
  local failures_before="$failures"
  local case_dir
  case_dir="$(prepare_case "$name" "$scenario" "$hash_kind")" || {
    fail "$name could not prepare fixture"
    return
  }

  if run_installer "$case_dir" "$scenario" >"$case_dir/output" 2>&1; then
    fail "$name unexpectedly installed"
  fi
  if [[ -s "$case_dir/tar.log" ]]; then
    fail "$name invoked tar before rejecting checksum metadata: $(cat "$case_dir/tar.log")"
  fi
  if [[ -s "$case_dir/install.log" ]]; then
    fail "$name invoked install after checksum rejection: $(cat "$case_dir/install.log")"
  fi
  if [[ -e "$case_dir/install/fojin" ]]; then
    fail "$name installed a binary after checksum rejection"
  fi
  if [[ "$failures" -eq "$failures_before" ]]; then
    printf 'PASS: %s failed before extraction\n' "$name"
  fi
}

expect_success "valid checksum via sha256sum" sha256sum
if [[ -n "$real_shasum" ]]; then
  expect_success "valid checksum via shasum fallback" shasum
else
  fail "test host does not provide shasum for fallback coverage"
fi

expect_failure_before_extract "missing SHA256SUMS download" download-failure
expect_failure_before_extract "missing target checksum record" missing-record
expect_failure_before_extract "duplicate target checksum record" duplicate-record
expect_failure_before_extract "valid plus malformed target candidate" valid-plus-marker-target
expect_failure_before_extract "valid plus star target candidate" valid-plus-star-target
expect_failure_before_extract "malformed checksum digest" malformed-digest
expect_failure_before_extract "non-hex checksum digest" non-hex-digest
expect_failure_before_extract "checksum record with extra field" extra-field
expect_failure_before_extract "wrong archive checksum" wrong-digest
expect_failure_before_extract "missing checksum utility" valid none
expect_failure_before_extract "hash output with extra field" valid hash-extra-field
expect_failure_before_extract "hash output with extra line" valid hash-extra-line

if ((failures != 0)); then
  printf '%d installer test(s) failed\n' "$failures" >&2
  exit 1
fi

printf 'installer script checks passed\n'
