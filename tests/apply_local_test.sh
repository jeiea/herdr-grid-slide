#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/herdr-grid-slide-apply-local.XXXXXX")
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

FAKE_BIN="$TMP_DIR/fake-bin"
mkdir -p "$FAKE_BIN"

cat >"$FAKE_BIN/cargo" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"$CARGO_CALLS"
[ "${FAIL_AT:-}" != build ] || exit 1
mkdir -p target/release
[ "${FAIL_AT:-}" != copy ] || exit 0
printf '%s\n' "$BINARY_CONTENT" >target/release/herdr-grid-slide
chmod +x target/release/herdr-grid-slide
EOF

cat >"$FAKE_BIN/herdr" <<'EOF'
#!/bin/sh
set -eu
test "$(cat bin/herdr-grid-slide)" = "$BINARY_CONTENT"
printf '%s|%s\n' "$PWD" "$*" >>"$HERDR_CALLS"
EOF
chmod +x "$FAKE_BIN/cargo" "$FAKE_BIN/herdr"

prepare_project() {
  PROJECT="$TMP_DIR/$1"
  mkdir -p "$PROJECT/.mise/tasks" "$PROJECT/scripts"
  PROJECT=$(CDPATH='' cd -- "$PROJECT" && pwd -P)
  cp "$ROOT/.mise/tasks/apply-local.ts" "$PROJECT/.mise/tasks/apply-local.ts"
  cp "$ROOT/scripts/task.ts" "$PROJECT/scripts/task.ts"
  CARGO_CALLS="$PROJECT/cargo-calls"
  HERDR_CALLS="$PROJECT/herdr-calls"
}

run_apply_local() {
  PATH="$FAKE_BIN:$PATH" \
    CARGO_CALLS="$CARGO_CALLS" HERDR_CALLS="$HERDR_CALLS" \
    BINARY_CONTENT="$1" FAIL_AT="${2:-}" \
    deno run -A "$PROJECT/.mise/tasks/apply-local.ts"
}

assert_no_staged_binary() {
  for staged in "$PROJECT"/bin/.herdr-grid-slide.*
  do
    if [ -e "$staged" ]; then
      echo "staged binary was not removed: $staged" >&2
      exit 1
    fi
  done
}

assert_failed_without_replacing_or_calling_herdr() {
  failure=$1
  prepare_project "failure-$failure"
  mkdir -p "$PROJECT/bin"
  printf '%s\n' existing >"$PROJECT/bin/herdr-grid-slide"

  if run_apply_local replacement "$failure"; then
    echo "$failure failure unexpectedly succeeded" >&2
    exit 1
  fi

  test "$(cat "$PROJECT/bin/herdr-grid-slide")" = existing
  test ! -s "$HERDR_CALLS"
  assert_no_staged_binary
}

prepare_project success
run_apply_local first
test "$(cat "$PROJECT/bin/herdr-grid-slide")" = first
test "$(cat "$PROJECT/target/release/herdr-grid-slide")" = first
test -x "$PROJECT/bin/herdr-grid-slide"
assert_no_staged_binary

run_apply_local second
test "$(cat "$PROJECT/bin/herdr-grid-slide")" = second
test "$(cat "$PROJECT/target/release/herdr-grid-slide")" = second
test "$(awk 'END { print NR }' "$CARGO_CALLS")" = 2
test "$(sort -u "$CARGO_CALLS")" = "build --locked --release"
test "$(cat "$HERDR_CALLS")" = "$(printf '%s\n' \
  "$PROJECT|plugin link ." \
  "$PROJECT|server reload-config" \
  "$PROJECT|plugin link ." \
  "$PROJECT|server reload-config")"
assert_no_staged_binary

assert_failed_without_replacing_or_calling_herdr build
assert_failed_without_replacing_or_calling_herdr copy

prepare_project failure-move
mkdir -p "$PROJECT/bin/herdr-grid-slide"
printf '%s\n' existing >"$PROJECT/bin/herdr-grid-slide/marker"
if run_apply_local replacement; then
  echo "move failure unexpectedly succeeded" >&2
  exit 1
fi
test "$(cat "$PROJECT/bin/herdr-grid-slide/marker")" = existing
test ! -s "$HERDR_CALLS"
assert_no_staged_binary
