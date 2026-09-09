#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/herdr-grid-slide-version.XXXXXX")
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

cp "$ROOT/Cargo.toml" "$TMP_DIR/Cargo.toml"
cp "$ROOT/Cargo.lock" "$TMP_DIR/Cargo.lock"
cp "$ROOT/herdr-plugin.toml" "$TMP_DIR/herdr-plugin.toml"
VERSION=$(awk -F ' *= *' '$1 == "version" { value = $2; gsub(/"/, "", value); print value; exit }' "$ROOT/herdr-plugin.toml")
NEXT_VERSION=$(printf '%s\n' "$VERSION" | awk -F . '{ print $1 "." $2 "." $3 + 1 }')

(cd "$TMP_DIR" && "$ROOT/scripts/check-version.sh")

awk -v next_version="$NEXT_VERSION" '
  /^\[\[package\]\]$/ { package = 1; root = 0 }
  package && /^name *= *"herdr-grid-slide"$/ { root = 1 }
  root && !changed && /^version *=/ {
    sub(/"[^"]+"/, "\"" next_version "\"")
    changed = 1
  }
  { print }
' "$TMP_DIR/Cargo.lock" >"$TMP_DIR/Cargo.lock.next"
mv "$TMP_DIR/Cargo.lock.next" "$TMP_DIR/Cargo.lock"
if (cd "$TMP_DIR" && "$ROOT/scripts/check-version.sh"); then
  echo "mismatched Cargo.lock version unexpectedly succeeded" >&2
  exit 1
fi
cp "$ROOT/Cargo.lock" "$TMP_DIR/Cargo.lock"

awk -v next_version="$NEXT_VERSION" '
  /^\[package\]$/ { package = 1 }
  /^\[/ && $0 != "[package]" { package = 0 }
  package && !changed && /^version *=/ {
    sub(/"[^"]+"/, "\"" next_version "\"")
    changed = 1
  }
  { print }
' "$TMP_DIR/Cargo.toml" >"$TMP_DIR/Cargo.toml.next"
mv "$TMP_DIR/Cargo.toml.next" "$TMP_DIR/Cargo.toml"
if (cd "$TMP_DIR" && "$ROOT/scripts/check-version.sh"); then
  echo "mismatched versions unexpectedly succeeded" >&2
  exit 1
fi

TAGGED="$TMP_DIR/tagged"
mkdir -p "$TAGGED/scripts" "$TAGGED/src"
cp "$ROOT/Cargo.toml" "$TAGGED/Cargo.toml"
cp "$ROOT/Cargo.lock" "$TAGGED/Cargo.lock"
cp "$ROOT/herdr-plugin.toml" "$TAGGED/herdr-plugin.toml"
cp "$ROOT/scripts/build-plugin.sh" "$TAGGED/scripts/build-plugin.sh"
cp "$ROOT/scripts/check-version.sh" "$TAGGED/scripts/check-version.sh"
cp "$ROOT/src/main.rs" "$TAGGED/src/main.rs"
(
  cd "$TAGGED"
  git init --quiet
  git config user.email test@example.invalid
  git config user.name "Version Test"
  git add .
  git commit --quiet -m initial
  git tag "v$VERSION"
  ./scripts/check-version.sh
  if ./scripts/check-version.sh --release; then
    echo "an existing release tag unexpectedly passed" >&2
    exit 1
  fi
  printf '\n// release input changed\n' >>src/main.rs
  if ./scripts/check-version.sh; then
    echo "changed release inputs unexpectedly reused a version" >&2
    exit 1
  fi
)
