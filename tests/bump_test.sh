#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/herdr-grid-slide-bump.XXXXXX")
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM
CURRENT_VERSION=$(awk -F ' *= *' '
  $1 == "version" { value = $2; gsub(/"/, "", value); print value; exit }
' "$ROOT/herdr-plugin.toml")
NEXT_VERSION=$(printf '%s\n' "$CURRENT_VERSION" | awk -F . '{ print $1 "." $2 "." $3 + 1 }')

prepare_project() {
  PROJECT="$TMP_DIR/$1"
  mkdir -p "$PROJECT/.mise/tasks" "$PROJECT/scripts" "$PROJECT/original"
  PROJECT=$(CDPATH='' cd -- "$PROJECT" && pwd)
  cp "$ROOT/.mise/tasks/bump.sh" "$PROJECT/.mise/tasks/bump.sh"
  cp "$ROOT/scripts/check-version.sh" "$PROJECT/scripts/check-version.sh"
  cp "$ROOT/herdr-plugin.toml" "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "$PROJECT"
  cp "$PROJECT/herdr-plugin.toml" "$PROJECT/Cargo.toml" "$PROJECT/Cargo.lock" \
    "$PROJECT/original"
}

run_bump() {
  (cd "$PROJECT" && "$PROJECT/.mise/tasks/bump.sh" "$@")
}

file_mode() {
  if stat -c '%a' "$1" 2>/dev/null; then
    return
  fi
  stat -f '%Lp' "$1"
}

assert_unchanged() {
  cmp "$PROJECT/original/herdr-plugin.toml" "$PROJECT/herdr-plugin.toml"
  cmp "$PROJECT/original/Cargo.toml" "$PROJECT/Cargo.toml"
  cmp "$PROJECT/original/Cargo.lock" "$PROJECT/Cargo.lock"
}

assert_no_temporary_files() {
  if find "$PROJECT" -maxdepth 1 \
    \( -name '*.bump.*' -o -name '*.backup.*' -o -name '.bump-validation.*' \) \
    -print | grep -q .
  then
    echo "bump temporary files were not removed" >&2
    exit 1
  fi
}

assert_rejected() {
  label=$1
  shift
  if run_bump "$@"; then
    echo "$label unexpectedly succeeded" >&2
    exit 1
  fi
  assert_unchanged
  assert_no_temporary_files
}

prepare_project success
chmod 600 "$PROJECT/herdr-plugin.toml"
chmod 640 "$PROJECT/Cargo.toml"
chmod 660 "$PROJECT/Cargo.lock"
plugin_mode=$(file_mode "$PROJECT/herdr-plugin.toml")
cargo_mode=$(file_mode "$PROJECT/Cargo.toml")
lock_mode=$(file_mode "$PROJECT/Cargo.lock")
run_bump "$NEXT_VERSION"
test "$(cd "$PROJECT" && ./scripts/check-version.sh)" = "$NEXT_VERSION"
awk -v next_version="$NEXT_VERSION" '
  /^\[/ { section = 1 }
  !section && /^version *=/ { sub(/"[^"]+"/, "\"" next_version "\"") }
  { print }
' "$PROJECT/original/herdr-plugin.toml" >"$PROJECT/expected-herdr-plugin.toml"
awk -v next_version="$NEXT_VERSION" '
  /^\[package\]$/ { package = 1 }
  /^\[/ && $0 != "[package]" { package = 0 }
  package && /^version *=/ { sub(/"[^"]+"/, "\"" next_version "\"") }
  { print }
' "$PROJECT/original/Cargo.toml" >"$PROJECT/expected-Cargo.toml"
awk -v next_version="$NEXT_VERSION" '
  /^\[\[package\]\]$/ { package = 1; root = 0 }
  package && /^name *= *"herdr-grid-slide"$/ { root = 1 }
  root && /^version *=/ { sub(/"[^"]+"/, "\"" next_version "\"") }
  { print }
' "$PROJECT/original/Cargo.lock" >"$PROJECT/expected-Cargo.lock"
cmp "$PROJECT/expected-herdr-plugin.toml" "$PROJECT/herdr-plugin.toml"
cmp "$PROJECT/expected-Cargo.toml" "$PROJECT/Cargo.toml"
cmp "$PROJECT/expected-Cargo.lock" "$PROJECT/Cargo.lock"
test "$(file_mode "$PROJECT/herdr-plugin.toml")" = "$plugin_mode"
test "$(file_mode "$PROJECT/Cargo.toml")" = "$cargo_mode"
test "$(file_mode "$PROJECT/Cargo.lock")" = "$lock_mode"
assert_no_temporary_files

prepare_project invalid-input
assert_rejected "missing version"
assert_rejected "extra argument" "$NEXT_VERSION" extra
for invalid_version in 1.2 1.2.3.4 01.2.3 1.02.3 1.2.03 v1.2.3 1.2.3-alpha
do
  assert_rejected "invalid version $invalid_version" "$invalid_version"
done
multiline_version=$(printf '%s\n%s' "$NEXT_VERSION" trailing)
if run_bump "$multiline_version" 2>"$PROJECT/multiline-error"; then
  echo "multiline version unexpectedly succeeded" >&2
  exit 1
fi
grep -F "version must be MAJOR.MINOR.PATCH without leading zeroes" \
  "$PROJECT/multiline-error" >/dev/null
assert_unchanged
assert_no_temporary_files

assert_rejected "current version" "$CURRENT_VERSION"

prepare_project validation-failure
printf '#!/bin/sh\nexit 1\n' >"$PROJECT/scripts/check-version.sh"
chmod +x "$PROJECT/scripts/check-version.sh"
assert_rejected "temporary validation failure" "$NEXT_VERSION"

prepare_project move-failure
FAKE_BIN="$PROJECT/fake-bin"
REAL_MV=$(command -v mv)
MV_CALLS="$PROJECT/mv-calls"
mkdir -p "$FAKE_BIN"
cat >"$FAKE_BIN/mv" <<'EOF'
#!/bin/sh
count=0
if [ -f "$MV_CALLS" ]; then
  count=$(cat "$MV_CALLS")
fi
count=$((count + 1))
printf '%s\n' "$count" >"$MV_CALLS"
if [ "$count" -eq "$FAIL_MV_AT" ]; then
  exit 1
fi
exec "$REAL_MV" "$@"
EOF
chmod +x "$FAKE_BIN/mv"
if (
  cd "$PROJECT"
  PATH="$FAKE_BIN:$PATH" REAL_MV="$REAL_MV" MV_CALLS="$MV_CALLS" FAIL_MV_AT=2 \
    "$PROJECT/.mise/tasks/bump.sh" "$NEXT_VERSION"
); then
  echo "intermediate move failure unexpectedly succeeded" >&2
  exit 1
fi
assert_unchanged
assert_no_temporary_files

prepare_project restore-failure
FAKE_BIN="$PROJECT/fake-bin"
REAL_MV=$(command -v mv)
mkdir -p "$FAKE_BIN"
cat >"$FAKE_BIN/mv" <<'EOF'
#!/bin/sh
case "$2:$3" in
  *Cargo.toml.bump.*:"$PROJECT/Cargo.toml") exit 1 ;;
  *herdr-plugin.toml.backup.*:"$PROJECT/herdr-plugin.toml") exit 1 ;;
esac
exec "$REAL_MV" "$@"
EOF
chmod +x "$FAKE_BIN/mv"
if (
  cd "$PROJECT"
  export REAL_MV PROJECT
  PATH="$FAKE_BIN:$PATH" "$PROJECT/.mise/tasks/bump.sh" "$NEXT_VERSION"
) 2>"$PROJECT/restore-error"; then
  echo "persistent restore failure unexpectedly succeeded" >&2
  exit 1
fi
set -- "$PROJECT"/herdr-plugin.toml.backup.*
test "$#" = 1
test -f "$1"
cmp "$PROJECT/original/herdr-plugin.toml" "$1"
grep -F "$1" "$PROJECT/restore-error" >/dev/null
cmp "$PROJECT/original/Cargo.toml" "$PROJECT/Cargo.toml"
cmp "$PROJECT/original/Cargo.lock" "$PROJECT/Cargo.lock"
if find "$PROJECT" -maxdepth 1 \
  \( -name '*.bump.*' -o -name '.bump-validation.*' \) -print | grep -q .
then
  echo "non-backup temporary files were not removed after restore failure" >&2
  exit 1
fi
