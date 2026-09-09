#!/bin/sh
set -eu

mode=${1:-check}
if [ "$mode" != "check" ] && [ "$mode" != "--release" ]; then
  echo "usage: $0 [--release]" >&2
  exit 2
fi

manifest_version=$(awk -F ' *= *' '$1 == "version" { value = $2; gsub(/"/, "", value); print value; exit }' herdr-plugin.toml)
cargo_version=$(awk '
  /^\[package\]$/ { package = 1; next }
  /^\[/ { package = 0 }
  package && /^version *=/ {
    value = $0
    sub(/^[^=]*= *"/, "", value)
    sub(/".*/, "", value)
    print value
    exit
  }
' Cargo.toml)
cargo_lock_version=$(awk '
  /^\[\[package\]\]$/ { package = 1; root = 0; next }
  package && /^name *= *"herdr-grid-slide"$/ { root = 1; next }
  root && /^version *=/ {
    value = $0
    sub(/^[^=]*= *"/, "", value)
    sub(/".*/, "", value)
    print value
    exit
  }
' Cargo.lock)

case "$manifest_version" in
  '' | *[!0-9.]* | .* | *.)
    echo "invalid plugin version: $manifest_version" >&2
    exit 1
    ;;
esac
if ! printf '%s\n' "$manifest_version" | awk -F . 'NF == 3 && $1 ~ /^(0|[1-9][0-9]*)$/ && $2 ~ /^(0|[1-9][0-9]*)$/ && $3 ~ /^(0|[1-9][0-9]*)$/ { found = 1 } END { exit !found }'; then
  echo "plugin version must be MAJOR.MINOR.PATCH: $manifest_version" >&2
  exit 1
fi
if [ "$manifest_version" != "$cargo_version" ]; then
  echo "version mismatch: herdr-plugin.toml=$manifest_version Cargo.toml=$cargo_version" >&2
  exit 1
fi
if [ "$manifest_version" != "$cargo_lock_version" ]; then
  echo "version mismatch: herdr-plugin.toml=$manifest_version Cargo.lock=$cargo_lock_version" >&2
  exit 1
fi

tag="v$manifest_version"
if [ "$mode" = "--release" ]; then
  if git show-ref --verify --quiet "refs/tags/$tag"; then
    echo "release tag already exists: $tag" >&2
    exit 1
  fi
  if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
    echo "remote release tag already exists: $tag" >&2
    exit 1
  else
    status=$?
    if [ "$status" -ne 2 ]; then
      echo "could not verify remote release tag: $tag" >&2
      exit "$status"
    fi
  fi
elif git rev-parse --git-dir >/dev/null 2>&1 && git show-ref --verify --quiet "refs/tags/$tag"; then
  if ! git diff --quiet "$tag" -- Cargo.toml Cargo.lock herdr-plugin.toml src scripts/build-plugin.sh .github/workflows/release.yml; then
    echo "release inputs changed since $tag; bump herdr-plugin.toml version" >&2
    exit 1
  fi
fi

printf '%s\n' "$manifest_version"
