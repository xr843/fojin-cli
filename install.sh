#!/bin/sh
# fojin-cli installer — downloads the prebuilt binary for this platform.
#
#   curl -fsSL https://raw.githubusercontent.com/xr843/fojin-cli/master/install.sh | sh
#
# Options (env vars):
#   FOJIN_INSTALL_DIR   install directory (default: ~/.local/bin)
#   FOJIN_VERSION       tag to install, e.g. v0.1.1 (default: latest v* release)
set -eu

REPO="xr843/fojin-cli"
INSTALL_DIR="${FOJIN_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "需要 curl,请先安装"
command -v tar >/dev/null 2>&1 || die "需要 tar,请先安装"

os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Linux)
    case "$arch" in
      x86_64) target="x86_64-unknown-linux-gnu" ;;
      *) die "暂无 Linux/$arch 预编译二进制,请改用: cargo install fojin-cli --locked" ;;
    esac ;;
  Darwin)
    case "$arch" in
      arm64|aarch64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) die "未知 macOS 架构: $arch" ;;
    esac ;;
  MINGW*|MSYS*|CYGWIN*)
    die "Windows 请从 Releases 页下载 zip: https://github.com/$REPO/releases/latest" ;;
  *)
    die "暂不支持 $os,请改用: cargo install fojin-cli --locked" ;;
esac

# Resolve version: newest v* tag (the repo also publishes data-v* releases,
# so /releases/latest alone is not reliable).
if [ -n "${FOJIN_VERSION:-}" ]; then
  version="$FOJIN_VERSION"
else
  version=$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=20" \
    | grep -o '"tag_name": *"v[0-9][^"]*"' | head -n1 | sed 's/.*"\(v[^"]*\)"/\1/')
  [ -n "$version" ] || die "无法获取最新版本号,请稍后重试或指定 FOJIN_VERSION=v0.1.1"
fi

asset="fojin-${version#v}-${target}.tar.gz"
release_url="https://github.com/$REPO/releases/download/$version"
url="$release_url/$asset"
checksums_url="$release_url/SHA256SUMS"

if command -v sha256sum >/dev/null 2>&1; then
  checksum_tool="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  checksum_tool="shasum"
else
  die "需要 sha256sum 或 shasum 来验证下载文件"
fi

say "下载 fojin $version ($target) ..."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/$asset" || die "下载失败: $url"
curl -fsSL "$checksums_url" -o "$tmp/SHA256SUMS" || die "下载失败: $checksums_url"

expected_checksum=$(awk -v asset="$asset" '
  {
    candidate = 0
    for (field = 1; field <= NF; field++) {
      if ($field == asset || $field == "*" asset) {
        candidate = 1
      }
    }
    if (!candidate) {
      next
    }

    candidates++
    if (NF != 2 || $2 != asset || length($1) != 64 || $1 ~ /[^0-9A-Fa-f]/) {
      malformed = 1
    } else {
      checksum = tolower($1)
    }
  }
  END {
    if (candidates != 1 || malformed) {
      exit 1
    }
    print checksum
  }
' "$tmp/SHA256SUMS") || die "SHA256SUMS 中缺少唯一且格式正确的 $asset 记录"

case "$checksum_tool" in
  sha256sum) checksum_output=$(sha256sum <"$tmp/$asset") || die "无法计算下载文件的 SHA-256" ;;
  shasum) checksum_output=$(shasum -a 256 <"$tmp/$asset") || die "无法计算下载文件的 SHA-256" ;;
esac
actual_checksum=$(printf '%s\n' "$checksum_output" | awk '
  {
    lines++
    if (NF != 2 || $2 != "-" || length($1) != 64 || $1 ~ /[^0-9A-Fa-f]/) {
      malformed = 1
    } else {
      checksum = tolower($1)
    }
  }
  END {
    if (lines != 1 || malformed) {
      exit 1
    }
    print checksum
  }
') || die "SHA-256 工具返回了格式错误的结果"
[ "$actual_checksum" = "$expected_checksum" ] || die "下载文件的 SHA-256 校验失败: $asset"

tar -xzf "$tmp/$asset" -C "$tmp"

mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/fojin-${version#v}-${target}/fojin" "$INSTALL_DIR/fojin"

say "已安装: $INSTALL_DIR/fojin ($("$INSTALL_DIR/fojin" --version))"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "提示: $INSTALL_DIR 不在 PATH 中,请把下面这行加进 shell 配置:"
     say "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
say '开始使用: fojin parallel "色即是空"'
