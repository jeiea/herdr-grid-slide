#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
SOURCE="$ROOT/target/release/herdr-grid-slide"
DESTINATION="$ROOT/bin/herdr-grid-slide"
STAGED="$ROOT/bin/.herdr-grid-slide.$$"

trap 'rm -f "$STAGED"' EXIT HUP INT TERM

cd "$ROOT"
cargo build --locked --release
mkdir -p "$ROOT/bin"
cp "$SOURCE" "$STAGED"
chmod +x "$STAGED"
mv -f "$STAGED" "$DESTINATION"
herdr plugin link .
