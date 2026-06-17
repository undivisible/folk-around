#!/bin/bash
set -euo pipefail

REPO="undivisible/folk-around"
BIN="${FOLK_AROUND_BIN:-/usr/local/bin/folk-around}"
VERSION="${1:-v0.3.4}"

case "$VERSION" in
  0.3.4|latest) VERSION="v0.3.4" ;;
  0.3.3) VERSION="v0.3.3" ;;
  0.3.2) VERSION="v0.3.2" ;;
  0.3.1) VERSION="v0.3.1" ;;
  0.3.0) VERSION="v0.3.0" ;;
esac

sha256_for_asset() {
  case "$1:$2" in
    v0.3.4:folk-around-darwin-aarch64) printf '%s\n' "6bccb05c0b618bde6b32d84306e3df8c0d0d6a0b0d5399c889c563c312477fe6" ;;
    v0.3.4:folk-around-darwin-x86_64) printf '%s\n' "8b2bf9ff51d84cabaa33cc120d9100ed368c200a199f9935c2c9761b66ce30f3" ;;
    v0.3.4:folk-around-linux-aarch64) printf '%s\n' "223f8e2172924e7521d93178302f2e0cbcb004f83d269e19ab338d9be8144010" ;;
    v0.3.4:folk-around-linux-x86_64) printf '%s\n' "9943758a2d7092731ce0878a2256ce4ebd575be4489d3a299a760045f8af2508" ;;
    v0.3.3:folk-around-darwin-aarch64) printf '%s\n' "407dd826a643c2972f4f92d8f65b4cf590b4de25d6bead9724b528370e66fb65" ;;
    v0.3.3:folk-around-darwin-x86_64) printf '%s\n' "7d4a9df52efbb2702b306b3cb16c80c48c1137f60837afbabf3958a11c0a91e6" ;;
    v0.3.3:folk-around-linux-aarch64) printf '%s\n' "6e68803d66328b2d67e8da806b718473fa471f94afcf1d18f919bb4819042df6" ;;
    v0.3.3:folk-around-linux-x86_64) printf '%s\n' "b181506e3628936eae0722cfda5ca21ed2e2d6922bb3e98bb5e7c7a1453f3201" ;;
    v0.3.2:folk-around-darwin-aarch64) printf '%s\n' "0b3ba3f7a7dcd2670bdc0759d12bae349713a3fb47f233fdede5b743b5ad1a64" ;;
    v0.3.2:folk-around-darwin-x86_64) printf '%s\n' "40396b90f55f6cfca6a3d931d7d85ab2b278c58861383c66e0ee857538cf18e0" ;;
    v0.3.2:folk-around-linux-aarch64) printf '%s\n' "3118c659c55d53fc7b609a9d1ba7e7f7030b3f98c48aea01f006c75676eff9e6" ;;
    v0.3.2:folk-around-linux-x86_64) printf '%s\n' "218f8179b095d35eaa7d95a94bc4b2c624244c8b8a4a6cdbc028da32eb01dee8" ;;
    v0.3.1:folk-around-darwin-aarch64) printf '%s\n' "4fffd0ebe4015c8dbb56c46d0888576ad5579c53f2052cb89ace1038f5c7f54e" ;;
    v0.3.1:folk-around-linux-aarch64) printf '%s\n' "eb4093762fabfdb1c4592dbb2e3b82185bd0cebcd97cdc7788bc2de3ec96f2b3" ;;
    v0.3.1:folk-around-linux-x86_64) printf '%s\n' "6b19653a12d3e93217cf1911eb709b35b10c186572a6e1ab570647a6875d91cd" ;;
    v0.3.0:folk-around-darwin-aarch64) printf '%s\n' "1915ae43fd8f67856e39b57f6f78006e14a08b3d8068a41df9f0f62ab4fb7171" ;;
    v0.3.0:folk-around-darwin-x86_64) printf '%s\n' "c73d124d64d7d9f4129650476efd44ad16bca156c61c1bbe5bfe55ac60bdeb8b" ;;
    v0.3.0:folk-around-linux-aarch64) printf '%s\n' "e8d28076c11d5800d8fc0151f11837318caf168f9c4116224ce39c499de5fd6d" ;;
    v0.3.0:folk-around-linux-x86_64) printf '%s\n' "b48f98d6d0e4f498eb822743a7e409add47f1e4b6d22b056538485ddb0551e8d" ;;
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
