#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
MANIFEST="$ROOT/herdr-plugin.toml"
DESTINATION="$ROOT/bin/herdr-move-pane"

version=$(awk -F ' *= *' '$1 == "version" { value = $2; gsub(/"/, "", value); print value; exit }' "$MANIFEST")
if [ -z "$version" ]; then
  echo "could not read version from herdr-plugin.toml" >&2
  exit 1
fi

case "$(uname -s):$(uname -m)" in
  Darwin:arm64) target=aarch64-apple-darwin ;;
  Darwin:x86_64) target=x86_64-apple-darwin ;;
  Linux:aarch64 | Linux:arm64) target=aarch64-unknown-linux-musl ;;
  Linux:x86_64 | Linux:amd64) target=x86_64-unknown-linux-musl ;;
  *)
    echo "unsupported platform: $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

mkdir -p "$ROOT/bin"
staged="$ROOT/bin/.herdr-move-pane.$$"
temporary=$(mktemp -d "${TMPDIR:-/tmp}/herdr-move-pane-download.XXXXXX")
trap 'rm -f "$staged"; rm -rf "$temporary"' EXIT HUP INT TERM

asset="herdr-move-pane-v${version}-${target}"
base_url=${HERDR_MOVE_PANE_RELEASE_BASE_URL:-"https://github.com/jeiea/herdr-move-pane/releases/download/v${version}"}

if [ -z "${HERDR_MOVE_PANE_RELEASE_BASE_URL:-}" ]; then
  curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    "$base_url/SHA256SUMS" --output "$temporary/SHA256SUMS"
  curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    "$base_url/$asset" --output "$temporary/$asset"
else
  curl --fail --location --silent --show-error \
    "$base_url/SHA256SUMS" --output "$temporary/SHA256SUMS"
  curl --fail --location --silent --show-error \
    "$base_url/$asset" --output "$temporary/$asset"
fi

if ! expected=$(awk -v asset="$asset" '
  $2 == asset || $2 == "*" asset {
    count++
    hash = $1
    if (NF != 2 || length(hash) != 64 || hash ~ /[^[:xdigit:]]/) invalid = 1
  }
  END {
    if (count != 1 || invalid) exit 1
    print tolower(hash)
  }
' "$temporary/SHA256SUMS"); then
  echo "SHA256SUMS must contain exactly one valid entry for $asset" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$temporary/$asset" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$temporary/$asset" | awk '{ print $1 }')
else
  echo "sha256sum or shasum is required" >&2
  exit 1
fi
if [ "$actual" != "$expected" ]; then
  echo "checksum mismatch for $asset" >&2
  exit 1
fi

cp "$temporary/$asset" "$staged"
chmod +x "$staged"
mv -f "$staged" "$DESTINATION"
