#!/bin/bash
set -euo pipefail

REPO="undivisible/folk-around"
BIN="/usr/local/bin/folk-around"
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
curl -fsSL -o "$BIN" "$URL"
chmod +x "$BIN"

echo " Installed to $BIN"
echo " Run: folk-around --help"