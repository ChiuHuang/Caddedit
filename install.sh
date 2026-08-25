#!/usr/bin/env bash
# Caddedit installer ??downloads the latest release binary for your platform.
#
#   curl -sSL https://raw.githubusercontent.com/ChiuHuang/Caddedit/main/install.sh | sudo bash
#
# Override destination: DEST=/usr/local/bin ./install.sh
set -euo pipefail

REPO="ChiuHuang/Caddedit"
DEST="${DEST:-/usr/local/bin}"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
    Linux) suffix="unknown-linux-musl" ;;
    Darwin) suffix="apple-darwin" ;;
    *) echo "unsupported OS: $os (use 'cargo install' on this platform)" >&2; exit 1 ;;
esac

case "$arch" in
    x86_64) target="x86_64-$suffix" ;;
    aarch64 | arm64) target="aarch64-$suffix" ;;
    *) echo "unsupported arch: $arch" >&2; exit 1 ;;
esac

asset="caddedit-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"

echo "==> downloading $url"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$tmp/$asset"
elif command -v wget >/dev/null 2>&1; then
    wget -qO "$tmp/$asset" "$url"
else
    echo "need curl or wget" >&2
    exit 1
fi

tar -xzf "$tmp/$asset" -C "$tmp"

if [ "$(id -u)" -ne 0 ] && [ ! -w "$DEST" ]; then
    echo "==> need permission to write $DEST (rerun with sudo)" >&2
    exit 1
fi

install -m 0755 "$tmp/caddedit" "$DEST/caddedit"

echo "==> installed $(caddedit --version)"
echo "    next:"
echo "      caddedit init     # split your Caddyfile"
echo "      caddedit ls       # see every route"
