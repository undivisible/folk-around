#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_SH="$SCRIPT_DIR/install.sh"

require_literal() {
  local text="$1"
  if ! grep -Fq "$text" "$INSTALL_SH"; then
    echo "missing: $text" >&2
    exit 1
  fi
}

reject_literal() {
  local text="$1"
  if grep -Fq "$text" "$INSTALL_SH"; then
    echo "unexpected: $text" >&2
    exit 1
  fi
}

require_regex() {
  local pattern="$1"
  if ! grep -Eq "$pattern" "$INSTALL_SH"; then
    echo "missing pattern: $pattern" >&2
    exit 1
  fi
}

require_literal 'TMP_BIN="$(mktemp'
require_literal 'trap '\''rm -f "$TMP_BIN" "$TMP_INSTALL"'\'' EXIT'
require_regex 'curl -fsSL -o "\$TMP_BIN" "\$URL"'
require_literal 'sha256_for_asset()'
require_literal 'v0.3.2:folk-around-linux-aarch64'
require_literal 'file_sha256()'
require_literal 'Checksum mismatch for $ASSET'
require_literal 'URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"'
require_regex 'install -m 755 "\$TMP_BIN" "\$TMP_INSTALL"'
require_regex 'sudo install -m 755 "\$TMP_BIN" "\$TMP_INSTALL"'
require_regex 'mv -f "\$TMP_INSTALL" "\$BIN"'
require_regex 'sudo mv -f "\$TMP_INSTALL" "\$BIN"'
reject_literal 'curl -fsSL -o "$BIN" "$URL"'
reject_literal 'releases/$VERSION/download'

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/folk-around-install-test.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT
MOCK_BIN="$WORK_DIR/bin"
mkdir -p "$MOCK_BIN"
cat > "$MOCK_BIN/uname" <<'MOCK'
#!/bin/bash
set -euo pipefail
case "$1" in
  -s) printf '%s\n' Darwin ;;
  -m) printf '%s\n' arm64 ;;
  *) exit 1 ;;
esac
MOCK
chmod +x "$MOCK_BIN/uname"

cat > "$MOCK_BIN/curl" <<'MOCK'
#!/bin/bash
set -euo pipefail
out=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o)
      out="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
printf '#!/bin/sh\necho installed\n' > "$out"
MOCK
chmod +x "$MOCK_BIN/curl"

cat > "$MOCK_BIN/shasum" <<'MOCK'
#!/bin/bash
set -euo pipefail
printf '%s  %s\n' "35b8c6be0b39def15e6e28ab11696139f4ec4c2801c6819f481c471f9c8bb1a0" "${@: -1}"
MOCK
chmod +x "$MOCK_BIN/shasum"

TARGET="$WORK_DIR/install/folk-around"
mkdir -p "$(dirname "$TARGET")"
PATH="$MOCK_BIN:$PATH" FOLK_AROUND_BIN="$TARGET" "$INSTALL_SH"

if [[ "$("$TARGET")" != "installed" ]]; then
  echo "installed binary did not execute" >&2
  exit 1
fi

if compgen -G "$(dirname "$TARGET")/.folk-around.*.tmp" > /dev/null; then
  echo "staged install file was not cleaned up" >&2
  exit 1
fi
