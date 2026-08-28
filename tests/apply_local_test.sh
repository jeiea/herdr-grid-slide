#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/herdr-move-pane-apply-local.XXXXXX")
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

REAL_CP=$(command -v cp)
REAL_CHMOD=$(command -v chmod)
REAL_MV=$(command -v mv)
FAKE_BIN="$TMP_DIR/fake-bin"
mkdir -p "$FAKE_BIN"

cat >"$FAKE_BIN/cargo" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"$CARGO_CALLS"
[ "${FAIL_AT:-}" != build ] || exit 1
mkdir -p target/release
printf '%s\n' "$BINARY_CONTENT" >target/release/herdr-move-pane
"$REAL_CHMOD" +x target/release/herdr-move-pane
EOF

cat >"$FAKE_BIN/cp" <<'EOF'
#!/bin/sh
"$REAL_CP" "$@"
[ "${FAIL_AT:-}" != copy ] || exit 1
EOF

cat >"$FAKE_BIN/mv" <<'EOF'
#!/bin/sh
[ "${FAIL_AT:-}" != move ] || exit 1
exec "$REAL_MV" "$@"
EOF

cat >"$FAKE_BIN/herdr" <<'EOF'
#!/bin/sh
test "$(cat bin/herdr-move-pane)" = "$BINARY_CONTENT"
printf '%s|%s\n' "$PWD" "$*" >>"$HERDR_CALLS"
EOF
chmod +x "$FAKE_BIN/cargo" "$FAKE_BIN/cp" "$FAKE_BIN/mv" "$FAKE_BIN/herdr"

prepare_project() {
  PROJECT="$TMP_DIR/$1"
  mkdir -p "$PROJECT/.mise/tasks"
  PROJECT=$(CDPATH='' cd -- "$PROJECT" && pwd)
  cp "$ROOT/.mise/tasks/apply-local.sh" "$PROJECT/.mise/tasks/apply-local.sh"
  CARGO_CALLS="$PROJECT/cargo-calls"
  HERDR_CALLS="$PROJECT/herdr-calls"
}

run_apply_local() {
  PATH="$FAKE_BIN:$PATH" \
    REAL_CP="$REAL_CP" REAL_CHMOD="$REAL_CHMOD" REAL_MV="$REAL_MV" \
    CARGO_CALLS="$CARGO_CALLS" HERDR_CALLS="$HERDR_CALLS" \
    BINARY_CONTENT="$1" FAIL_AT="${2:-}" \
    "$PROJECT/.mise/tasks/apply-local.sh"
}

assert_no_staged_binary() {
  for staged in "$PROJECT"/bin/.herdr-move-pane.*
  do
    if [ -e "$staged" ]; then
      echo "staged binary was not removed: $staged" >&2
      exit 1
    fi
  done
}

assert_failed_without_replacing_or_linking() {
  failure=$1
  prepare_project "failure-$failure"
  mkdir -p "$PROJECT/bin"
  printf '%s\n' existing >"$PROJECT/bin/herdr-move-pane"

  if run_apply_local replacement "$failure"; then
    echo "$failure failure unexpectedly succeeded" >&2
    exit 1
  fi

  test "$(cat "$PROJECT/bin/herdr-move-pane")" = existing
  test ! -s "$HERDR_CALLS"
  assert_no_staged_binary
}

prepare_project success
run_apply_local first
test "$(cat "$PROJECT/bin/herdr-move-pane")" = first
test "$(cat "$PROJECT/target/release/herdr-move-pane")" = first
test -x "$PROJECT/bin/herdr-move-pane"
assert_no_staged_binary

run_apply_local second
test "$(cat "$PROJECT/bin/herdr-move-pane")" = second
test "$(cat "$PROJECT/target/release/herdr-move-pane")" = second
test "$(awk 'END { print NR }' "$CARGO_CALLS")" = 2
test "$(awk 'END { print NR }' "$HERDR_CALLS")" = 2
test "$(sort -u "$CARGO_CALLS")" = "build --locked --release"
test "$(sort -u "$HERDR_CALLS")" = "$PROJECT|plugin link ."
assert_no_staged_binary

assert_failed_without_replacing_or_linking build
assert_failed_without_replacing_or_linking copy
assert_failed_without_replacing_or_linking move
