#!/bin/sh
#MISE description="Update the plugin and Cargo versions to MAJOR.MINOR.PATCH"
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: mise run bump -- <MAJOR.MINOR.PATCH>" >&2
  exit 2
fi

next_version=$1
if ! printf '%s\n' "$next_version" | awk -F . '
  { lines++ }
  NF == 3 &&
  $1 ~ /^(0|[1-9][0-9]*)$/ &&
  $2 ~ /^(0|[1-9][0-9]*)$/ &&
  $3 ~ /^(0|[1-9][0-9]*)$/ { valid = 1 }
  END { exit !(lines == 1 && valid) }
'; then
  echo "version must be MAJOR.MINOR.PATCH without leading zeroes: $next_version" >&2
  exit 2
fi

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
current_version=$(awk -F ' *= *' '
  $1 == "version" { value = $2; gsub(/"/, "", value); print value; exit }
' "$ROOT/herdr-plugin.toml")
if [ "$next_version" = "$current_version" ]; then
  echo "version is already $next_version" >&2
  exit 1
fi

suffix=".bump.$$"
backup_suffix=".backup.$$"
plugin_stage="$ROOT/herdr-plugin.toml$suffix"
cargo_stage="$ROOT/Cargo.toml$suffix"
lock_stage="$ROOT/Cargo.lock$suffix"
plugin_backup="$ROOT/herdr-plugin.toml$backup_suffix"
cargo_backup="$ROOT/Cargo.toml$backup_suffix"
lock_backup="$ROOT/Cargo.lock$backup_suffix"
validation=
restore_needed=0

cleanup() {
  status=$?
  set +e
  if [ "$restore_needed" -eq 1 ]; then
    restore_failed=0
    if [ -f "$plugin_backup" ]; then
      if ! mv -f "$plugin_backup" "$ROOT/herdr-plugin.toml"; then
        echo "failed to restore herdr-plugin.toml; backup retained at $plugin_backup" >&2
        restore_failed=1
      fi
    fi
    if [ -f "$cargo_backup" ]; then
      if ! mv -f "$cargo_backup" "$ROOT/Cargo.toml"; then
        echo "failed to restore Cargo.toml; backup retained at $cargo_backup" >&2
        restore_failed=1
      fi
    fi
    if [ -f "$lock_backup" ]; then
      if ! mv -f "$lock_backup" "$ROOT/Cargo.lock"; then
        echo "failed to restore Cargo.lock; backup retained at $lock_backup" >&2
        restore_failed=1
      fi
    fi
    if [ "$restore_failed" -ne 0 ]; then
      status=1
    fi
  else
    rm -f "$plugin_backup" "$cargo_backup" "$lock_backup"
  fi
  rm -f "$plugin_stage" "$cargo_stage" "$lock_stage"
  if [ -n "$validation" ]; then
    rm -rf "$validation"
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

cp -p "$ROOT/herdr-plugin.toml" "$plugin_stage"
awk -v next_version="$next_version" '
  /^\[/ { section = 1 }
  !section && /^version *=/ {
    sub(/"[^"]+"/, "\"" next_version "\"")
    changed++
  }
  { print }
  END { if (changed != 1) exit 1 }
' "$ROOT/herdr-plugin.toml" >"$plugin_stage"

cp -p "$ROOT/Cargo.toml" "$cargo_stage"
awk -v next_version="$next_version" '
  /^\[package\]$/ { package = 1 }
  /^\[/ && $0 != "[package]" { package = 0 }
  package && /^version *=/ {
    sub(/"[^"]+"/, "\"" next_version "\"")
    changed++
  }
  { print }
  END { if (changed != 1) exit 1 }
' "$ROOT/Cargo.toml" >"$cargo_stage"

cp -p "$ROOT/Cargo.lock" "$lock_stage"
awk -v next_version="$next_version" '
  /^\[\[package\]\]$/ { package = 1; root = 0 }
  package && /^name *= *"herdr-grid-slide"$/ { root = 1 }
  root && /^version *=/ {
    sub(/"[^"]+"/, "\"" next_version "\"")
    changed++
  }
  { print }
  END { if (changed != 1) exit 1 }
' "$ROOT/Cargo.lock" >"$lock_stage"

validation=$(mktemp -d "$ROOT/.bump-validation.XXXXXX")
cp "$plugin_stage" "$validation/herdr-plugin.toml"
cp "$cargo_stage" "$validation/Cargo.toml"
cp "$lock_stage" "$validation/Cargo.lock"
if ! (cd "$validation" && "$ROOT/scripts/check-version.sh" >/dev/null); then
  echo "generated version files failed validation" >&2
  exit 1
fi

cp -p "$ROOT/herdr-plugin.toml" "$plugin_backup"
cp -p "$ROOT/Cargo.toml" "$cargo_backup"
cp -p "$ROOT/Cargo.lock" "$lock_backup"
restore_needed=1

if ! mv -f "$plugin_stage" "$ROOT/herdr-plugin.toml"; then
  echo "failed to replace herdr-plugin.toml" >&2
  exit 1
fi
if ! mv -f "$cargo_stage" "$ROOT/Cargo.toml"; then
  echo "failed to replace Cargo.toml" >&2
  exit 1
fi
if ! mv -f "$lock_stage" "$ROOT/Cargo.lock"; then
  echo "failed to replace Cargo.lock" >&2
  exit 1
fi

restore_needed=0
printf '%s\n' "$next_version"
