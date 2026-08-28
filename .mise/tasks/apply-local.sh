#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
SOURCE="$ROOT/target/release/herdr-move-pane"
DESTINATION="$ROOT/bin/herdr-move-pane"
STAGED="$ROOT/bin/.herdr-move-pane.$$"

trap 'rm -f "$STAGED"' EXIT HUP INT TERM

cd "$ROOT"
cargo build --locked --release
mkdir -p "$ROOT/bin"
cp "$SOURCE" "$STAGED"
chmod +x "$STAGED"
mv -f "$STAGED" "$DESTINATION"
herdr plugin link .
