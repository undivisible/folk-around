#!/bin/bash
set -euo pipefail

REPO="undivisible/folk-around"
BIN="${FOLK_AROUND_BIN:-/usr/local/bin/folk-around}"
VERSION="${1:-latest}"

echo " Installing folk-around..."

# Detect OS/arch
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)  OS="darwin" ;;
  Linux)   OS="linux" ;;
  *)       echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)          echo "Unsupported arch: $ARCH"; exit 1 ;;
esac

# Download
URL="https://github.com/$REPO/releases/$VERSION/download/folk-around-$OS-$ARCH"
echo " Downloading $URL..."
TMP_BIN="$(mktemp "${TMPDIR:-/tmp}/folk-around.XXXXXX")"
TMP_INSTALL="$(dirname "$BIN")/.folk-around.$$.tmp"
trap 'rm -f "$TMP_BIN" "$TMP_INSTALL"' EXIT
curl -fsSL -o "$TMP_BIN" "$URL"
chmod 755 "$TMP_BIN"

if [[ -w "$(dirname "$BIN")" && ( ! -e "$BIN" || -w "$BIN" ) ]]; then
  install -m 755 "$TMP_BIN" "$TMP_INSTALL"
  mv -f "$TMP_INSTALL" "$BIN"
else
  sudo install -m 755 "$TMP_BIN" "$TMP_INSTALL"
  sudo mv -f "$TMP_INSTALL" "$BIN"
fi

echo " Installed to $BIN"
echo " Run: folk-around --help"
