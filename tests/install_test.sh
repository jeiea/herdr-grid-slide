#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/herdr-move-pane-install.XXXXXX")
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

PROJECT="$TMP_DIR/project"
RELEASE="$TMP_DIR/release"
FAKE_BIN="$TMP_DIR/fake-bin"
mkdir -p "$PROJECT/scripts" "$RELEASE" "$FAKE_BIN"
cp "$ROOT/herdr-plugin.toml" "$PROJECT/herdr-plugin.toml"
cp "$ROOT/scripts/build-plugin.sh" "$PROJECT/scripts/build-plugin.sh"

VERSION=$(awk -F ' *= *' '$1 == "version" { value = $2; gsub(/"/, "", value); print value; exit }' "$PROJECT/herdr-plugin.toml")
for TARGET in \
  aarch64-apple-darwin \
  x86_64-apple-darwin \
  aarch64-unknown-linux-musl \
  x86_64-unknown-linux-musl
do
  ASSET="herdr-move-pane-v${VERSION}-${TARGET}"
  printf '#!/bin/sh\nprintf "%s\\n"\n' "$TARGET" >"$RELEASE/$ASSET"
  chmod +x "$RELEASE/$ASSET"
done
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$RELEASE" && sha256sum herdr-move-pane-* >SHA256SUMS)
else
  (cd "$RELEASE" && shasum -a 256 herdr-move-pane-* >SHA256SUMS)
fi

cat >"$FAKE_BIN/uname" <<'EOF'
#!/bin/sh
case "$1" in
  -s) printf '%s\n' "$MOCK_UNAME_S" ;;
  -m) printf '%s\n' "$MOCK_UNAME_M" ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$FAKE_BIN/uname"

while read -r SYSTEM MACHINE TARGET
do
  PATH="$FAKE_BIN:$PATH" MOCK_UNAME_S="$SYSTEM" MOCK_UNAME_M="$MACHINE" \
    HERDR_MOVE_PANE_RELEASE_BASE_URL="file://$RELEASE" "$PROJECT/scripts/build-plugin.sh"
  test "$("$PROJECT/bin/herdr-move-pane")" = "$TARGET"
done <<'EOF'
Darwin arm64 aarch64-apple-darwin
Darwin x86_64 x86_64-apple-darwin
Linux aarch64 aarch64-unknown-linux-musl
Linux x86_64 x86_64-unknown-linux-musl
EOF

if PATH="$FAKE_BIN:$PATH" MOCK_UNAME_S=Windows MOCK_UNAME_M=x86_64 \
  HERDR_MOVE_PANE_RELEASE_BASE_URL="file://$RELEASE" "$PROJECT/scripts/build-plugin.sh"; then
  echo "unsupported platform unexpectedly succeeded" >&2
  exit 1
fi

printf '#!/bin/sh\nprintf "existing binary\\n"\n' >"$PROJECT/bin/herdr-move-pane"
chmod +x "$PROJECT/bin/herdr-move-pane"
TARGET=aarch64-apple-darwin
ASSET="herdr-move-pane-v${VERSION}-${TARGET}"
printf 'corrupted asset\n' >"$RELEASE/$ASSET"

if PATH="$FAKE_BIN:$PATH" MOCK_UNAME_S=Darwin MOCK_UNAME_M=arm64 \
  HERDR_MOVE_PANE_RELEASE_BASE_URL="file://$RELEASE" "$PROJECT/scripts/build-plugin.sh"; then
  echo "checksum mismatch unexpectedly succeeded" >&2
  exit 1
fi
test "$("$PROJECT/bin/herdr-move-pane")" = "existing binary"

if command -v sha256sum >/dev/null 2>&1; then
  CHECKSUM=$(sha256sum "$RELEASE/$ASSET" | awk '{ print $1 }')
else
  CHECKSUM=$(shasum -a 256 "$RELEASE/$ASSET" | awk '{ print $1 }')
fi
printf '%s  %s\n%s  %s\n' "$CHECKSUM" "$ASSET" "$CHECKSUM" "$ASSET" >"$RELEASE/SHA256SUMS"
if PATH="$FAKE_BIN:$PATH" MOCK_UNAME_S=Darwin MOCK_UNAME_M=arm64 \
  HERDR_MOVE_PANE_RELEASE_BASE_URL="file://$RELEASE" "$PROJECT/scripts/build-plugin.sh"; then
  echo "duplicate checksum entries unexpectedly succeeded" >&2
  exit 1
fi
test "$("$PROJECT/bin/herdr-move-pane")" = "existing binary"
