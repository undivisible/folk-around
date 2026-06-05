#!/bin/bash
set -euo pipefail

REPO="undivisible/folk-around"
BIN="${FOLK_AROUND_BIN:-/usr/local/bin/folk-around}"
VERSION="${1:-v0.3.0}"

case "$VERSION" in
  0.3.0|latest) VERSION="v0.3.0" ;;
esac

sha256_for_asset() {
  case "$1:$2" in
    v0.3.0:folk-around-darwin-aarch64) printf '%s\n' "690b0fff1e719bc47534d35e4ac62426f9138c599bca276171c640c830b29aa2" ;;
    v0.3.0:folk-around-darwin-x86_64) printf '%s\n' "421c599ce1b57060b825eeb498a2e5196b8c5bb1f14a0b7db39934ecba51dca5" ;;
    v0.3.0:folk-around-linux-aarch64) printf '%s\n' "9284f392a4b01c02c94f40baceb08470e6345a631126b2d2d88a169971711fd7" ;;
    v0.3.0:folk-around-linux-x86_64) printf '%s\n' "774ebce1406e1b95f15172832b020dd31b12948e6c3f19eee04c80dba8ee2da8" ;;
    *) return 1 ;;
  esac
}

file_sha256() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "shasum or sha256sum is required" >&2
    exit 1
  fi
}

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

ASSET="folk-around-$OS-$ARCH"
if ! EXPECTED_SHA256="$(sha256_for_asset "$VERSION" "$ASSET")"; then
  echo "No checksum for $VERSION/$ASSET" >&2
  exit 1
fi

# Download
URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
echo " Downloading $URL..."
TMP_BIN="$(mktemp "${TMPDIR:-/tmp}/folk-around.XXXXXX")"
TMP_INSTALL="$(dirname "$BIN")/.folk-around.$$.tmp"
trap 'rm -f "$TMP_BIN" "$TMP_INSTALL"' EXIT
curl -fsSL -o "$TMP_BIN" "$URL"
ACTUAL_SHA256="$(file_sha256 "$TMP_BIN")"
if [[ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]]; then
  echo "Checksum mismatch for $ASSET" >&2
  echo "expected: $EXPECTED_SHA256" >&2
  echo "actual:   $ACTUAL_SHA256" >&2
  exit 1
fi
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
